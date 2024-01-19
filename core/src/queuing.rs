use std::ops::{Deref, DerefMut};
use std::os::fd::RawFd;
use std::os::fd::{AsRawFd, OwnedFd};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use a653rs::bindings::PortDirection;
use itertools::Itertools;
use memfd::{FileSeal, Memfd, MemfdOptions};
use memmap2::MmapMut;

use crate::channel::{PortConfig, QueuingChannelConfig};
use crate::error::{ResultExt, SystemError, TypedError, TypedResult};
use crate::partition::QueuingConstant;

/// A datagram is the Rust representation of the mapped memory of a queuing port.
/// It can be parsed from the its in-memory data and will then contain both references
/// into the shared memory and additional information such as the size per message.
///
/// It is important, that the datagram does not copy its data from the mapped memory.
/// Instead it tries to contain references into the mapped memory to prevent unnecessary allocations.
///
/// # Memory layout of mapped memory
/// - `usize`: Size per msg
/// - `usize`: Num of max msgs (capacity)
/// - `AtomicUsize`: Current read index
/// - `AtomicUsize`: Current number of messages in the queue
/// - List of messages:
///   - `usize`: Data size of this message
///   - `[u8; msg_size]`: Data of this message
#[derive(Debug)]
struct Datagram<'m> {
    msg_size: usize,
    msg_capacity: usize,
    read_idx: &'m AtomicUsize,
    current_num_msgs: &'m AtomicUsize,
    /// As of now mutable references are stored
    /// Possible improvement: Maybe each message can be wrapped into its own [Mutex]. This would sacrifice some performance and memory for a little more safety when multiple processes are working on the same memory.
    messages: Vec<Message<'m>>,
}

#[derive(Debug)]
struct Message<'m> {
    len: &'m mut usize,
    data: &'m mut [u8], // this slice is actually of length msg_size which will be equal for all messages in a queue
}

impl<'a> Message<'a> {
    /// Bytes must contain a usize length starting from index 0 followed by at least `length` bytes
    unsafe fn from_bytes(bytes: &'a mut [u8]) -> Self {
        let (len, data) = bytes.split_at_mut(std::mem::size_of::<usize>());
        let len = (len.as_ptr() as *mut usize).as_mut().unwrap();

        Self { len, data }
    }

    fn get_data(&self) -> &[u8] {
        &self.data[0..*self.len]
    }

    /// Returns number of bytes written
    fn set_data(&mut self, new_data: &[u8]) -> usize {
        let len = self.data.len().min(new_data.len());
        self.data[..len].copy_from_slice(&new_data[..len]);
        *self.len = len;
        len
    }

    fn copy_from_msg(&mut self, source: &Message) {
        debug_assert_eq!(self.data.len(), source.data.len());

        *self.len = *source.len;
        self.data.copy_from_slice(source.data);
    }
}

impl Datagram<'_> {
    fn size(msg_size: usize, msg_num: usize) -> usize {
        use std::mem::size_of;

        size_of::<usize>() // Size per msg
        + size_of::<usize>() // Msg num capacity
        + size_of::<AtomicUsize>() // Current read index
        + size_of::<AtomicUsize>() // Current number of messages in the queue
        + msg_num * (size_of::<usize>() + msg_size) // Message length header and contents
    }

    fn init_at(mmap: &mut MmapMut, msg_size: usize, queue_capacity: usize) {
        let mmap_bytes = (*mmap).as_mut();

        // Split mmap bytes slice into the contained fields
        let (msg_size_ref, rest) = unsafe { Self::strip_field_mut::<usize>(mmap_bytes) };
        let (queue_capacity_ref, rest) = unsafe { Self::strip_field_mut::<usize>(rest) };
        let (read_idx, rest) = unsafe { Self::strip_field_mut::<AtomicUsize>(rest) };
        let (current_num_msgs, _) = unsafe { Self::strip_field_mut::<AtomicUsize>(rest) };

        *msg_size_ref = msg_size;
        *queue_capacity_ref = queue_capacity;

        unsafe {
            ptr::write(read_idx as *mut AtomicUsize, AtomicUsize::new(0));
        }
        unsafe {
            ptr::write(current_num_msgs as *mut AtomicUsize, AtomicUsize::new(0));
        }
    }

    unsafe fn strip_field_mut<T>(bytes: &mut [u8]) -> (&mut T, &mut [u8]) {
        debug_assert!(bytes.len() >= std::mem::size_of::<T>());
        let (field, rest) = bytes.split_at_mut(std::mem::size_of::<T>());
        let field = (field.as_ptr() as *mut T).as_mut().unwrap();
        (field, rest)
    }

    fn read(mmap: &mut MmapMut) -> Datagram {
        /// Unsafe as it is required for `bytes` to contain a T starting from index 0
        // Get the bytes from the mapped memory
        let mmap_bytes = (*mmap).as_mut();

        // Split mmap bytes slice into the contained fields
        let (&mut msg_size, rest) = unsafe { Self::strip_field_mut::<usize>(mmap_bytes) };
        let (&mut queue_capacity, rest) = unsafe { Self::strip_field_mut::<usize>(rest) };
        let (read_idx, rest) = unsafe { Self::strip_field_mut::<AtomicUsize>(rest) };
        let (current_num_msgs, mut msg_data) =
            unsafe { Self::strip_field_mut::<AtomicUsize>(rest) };

        let msg_size_with_header = msg_size + std::mem::size_of::<usize>();

        debug_assert_eq!(msg_size_with_header * queue_capacity, msg_data.len());
        let mut messages = Vec::with_capacity(queue_capacity);
        for _ in 0..queue_capacity {
            let (message_data, rest) = msg_data.split_at_mut(msg_size_with_header);
            unsafe {
                messages.push(Message::from_bytes(message_data));
            }
            msg_data = rest;
        }

        // TODO: do check if data has changed? is this needed?

        Datagram {
            msg_size,
            msg_capacity: queue_capacity,
            current_num_msgs: &*current_num_msgs,
            read_idx: &*read_idx,
            messages,
        }
    }
}

#[derive(Debug)]
pub struct Queuing {
    msg_size: usize,
    max_num_msg: usize,

    source_receiver: MmapMut,
    source: OwnedFd,
    source_port: PortConfig,

    destination_sender: MmapMut,
    destination: OwnedFd,
    destination_port: PortConfig,

    last_num_msgs_in_source: usize,
}

impl TryFrom<QueuingChannelConfig> for Queuing {
    type Error = TypedError;

    fn try_from(config: QueuingChannelConfig) -> Result<Self, Self::Error> {
        let msg_size = config.msg_size.as_u64() as usize;
        let msg_num = config.msg_num;

        let source_port_name = config.source.name();
        let (source_receiver, source) = Self::source(
            format!("queuing_{source_port_name}_source"),
            msg_size,
            config.msg_num,
        )?;
        let (destination_sender, destination) = Self::destination(
            format!("queuing_{source_port_name}_destination"),
            msg_size,
            config.msg_num,
        )?;

        // TODO: some checks?

        Ok(Self {
            msg_size,
            max_num_msg: msg_num,
            source_receiver,
            source,
            source_port: config.source,
            destination_sender,
            destination,
            destination_port: config.destination,
            last_num_msgs_in_source: 0,
        })
    }
}

impl Queuing {
    pub fn constant(&self, part: impl AsRef<str>) -> Option<QueuingConstant> {
        let (dir, fd, port) = if self.source_port.partition.eq(part.as_ref()) {
            (
                PortDirection::Source,
                self.source_fd(),
                &self.source_port.port,
            )
        } else {
            (
                PortDirection::Destination,
                self.destination_fd(),
                &self.destination_port.port,
            )
        };

        Some(QueuingConstant {
            name: port.clone(),
            dir,
            msg_size: self.msg_size,
            max_num_msg: self.max_num_msg,
            fd,
        })
    }

    pub fn name(&self) -> String {
        format!("{}:{}", &self.source_port.partition, self.source_port.port)
    }

    fn memfd(name: impl AsRef<str>, msg_size: usize, max_num_msgs: usize) -> TypedResult<Memfd> {
        let size = Datagram::size(msg_size, max_num_msgs);

        let mem = MemfdOptions::default()
            .close_on_exec(false)
            .allow_sealing(true)
            .create(name)
            .typ(SystemError::Panic)?;
        mem.as_file().set_len(size as u64).typ(SystemError::Panic)?;
        mem.add_seals(&[FileSeal::SealShrink, FileSeal::SealGrow])
            .typ(SystemError::Panic)?;

        Ok(mem)
    }

    fn source(
        name: impl AsRef<str>,
        msg_size: usize,
        max_num_msgs: usize,
    ) -> TypedResult<(MmapMut, OwnedFd)> {
        let mem = Self::memfd(name, msg_size, max_num_msgs)?;

        let mut mmap = unsafe { MmapMut::map_mut(mem.as_raw_fd()).typ(SystemError::Panic)? };

        mem.add_seals(&[FileSeal::SealSeal])
            .typ(SystemError::Panic)?;

        Datagram::init_at(&mut mmap, msg_size, max_num_msgs);

        Ok((mmap, mem.into_file().into()))
    }

    fn destination(
        name: impl AsRef<str>,
        msg_size: usize,
        max_num_msgs: usize,
    ) -> TypedResult<(MmapMut, OwnedFd)> {
        let mem = Self::memfd(name, msg_size, max_num_msgs)?;

        let mut mmap = unsafe { MmapMut::map_mut(mem.as_raw_fd()).typ(SystemError::Panic)? };

        mem.add_seals(&[FileSeal::SealSeal])
            .typ(SystemError::Panic)?;

        Datagram::init_at(&mut mmap, msg_size, max_num_msgs);

        Ok((mmap, mem.into_file().into()))
    }

    fn perform_swap(mut source_datagram: Datagram, mut destination_datagram: Datagram) {
        // First get some necessary values for both source and destination
        debug_assert_eq!(
            source_datagram.msg_capacity,
            destination_datagram.msg_capacity
        );
        let capacity = source_datagram.msg_capacity;

        let read_idx_source = source_datagram.read_idx.load(Ordering::Acquire);
        let read_idx_destination = destination_datagram.read_idx.load(Ordering::Acquire);

        let num_msgs_in_destination = destination_datagram
            .current_num_msgs
            .load(Ordering::Acquire);
        let num_msgs_in_source = source_datagram.current_num_msgs.load(Ordering::Acquire);

        let write_idx_source = (read_idx_source + num_msgs_in_source) % capacity;
        let write_idx_destination = (read_idx_destination + num_msgs_in_destination) % capacity;

        let num_msgs_read_by_destination =
            ((read_idx_destination + capacity) - read_idx_source) % capacity;
        let num_msgs_written_by_source =
            ((write_idx_source + capacity) - write_idx_destination) % capacity;

        // Now we will perform the actual swap in three ordered steps:
        // 1. Shorten length of source and advance its read_idx by the same amount, because the destination may have advanced
        //    its read_idx through reading messages.
        // 2. Copy all new messages from source to destination.
        //    This includes all messages in range `destination.write_idx..source.write_idx`.
        // 3. Increase length of destination, because the source may have more messages now through sending messages.

        // Step 1.
        // Add capacity before modulo to prevent usize underflow
        source_datagram.read_idx.swap(
            read_idx_destination, //(num_msgs_in_source + num_msgs_read_by_destination) % capacity,
            Ordering::AcqRel,
        );
        source_datagram.current_num_msgs.swap(
            num_msgs_in_source - num_msgs_read_by_destination,
            Ordering::AcqRel,
        );

        // Step 2.
        // Now read_idx_destination=read_idx_source
        let new_message_indices = (write_idx_destination..(write_idx_source + capacity))
            .map(|i| i % source_datagram.msg_capacity);
        new_message_indices.for_each(|msg_idx| {
            let src_msg = &source_datagram.messages[msg_idx];
            let mut destination_msg = &mut destination_datagram.messages[msg_idx];

            destination_msg.copy_from_msg(src_msg)
        });

        // Step 3.
        let _ = destination_datagram.current_num_msgs.fetch_update(
            Ordering::Release,
            Ordering::Acquire,
            |x| Some((x + num_msgs_written_by_source) % capacity),
        );
    }

    pub fn swap(&mut self) -> bool {
        let source_datagram = Datagram::read(&mut self.source_receiver);
        let num_msgs_in_source = source_datagram.current_num_msgs.load(Ordering::Acquire);

        // Check if there are any new messages
        if self.last_num_msgs_in_source == num_msgs_in_source {
            return false;
        }
        self.last_num_msgs_in_source = num_msgs_in_source;

        // Parse destination datagram
        let destination_datagram = Datagram::read(&mut self.destination_sender);

        Self::perform_swap(source_datagram, destination_datagram);

        true
    }

    pub fn source_fd(&self) -> RawFd {
        self.source.as_raw_fd()
    }
    pub fn destination_fd(&self) -> RawFd {
        self.destination.as_raw_fd()
    }
}

#[derive(Debug)]
pub struct QueuingSource(MmapMut);

impl QueuingSource {
    pub fn write(&mut self, data: &[u8]) -> usize {
        let mut datagram = Datagram::read(&mut self.0);
        let read_idx = datagram.read_idx.load(Ordering::Acquire);
        let current_num_msgs = datagram.current_num_msgs.load(Ordering::Acquire);

        // Check if queue is full
        if current_num_msgs == datagram.msg_capacity {
            trace!(
                "tried to write to full queue, maximum capacity of messages is {}",
                datagram.msg_capacity
            );
            return 0;
        }

        datagram.current_num_msgs.fetch_add(1, Ordering::Release);

        let write_idx = (read_idx + current_num_msgs) % datagram.msg_capacity;

        let message = &mut datagram.messages[write_idx];

        message.set_data(data)
    }
}

impl TryFrom<RawFd> for QueuingSource {
    type Error = TypedError;

    fn try_from(file: RawFd) -> Result<Self, Self::Error> {
        let mmap = unsafe { MmapMut::map_mut(file).typ(SystemError::Panic)? };

        Ok(Self(mmap))
    }
}

impl QueuingDestination {
    /// Reads the current message from the queue into a buffer and increments the current read index.
    /// Returns the number of bytes read.
    pub fn read(&mut self, data: &mut [u8]) -> usize {
        let datagram = Datagram::read(&mut self.0);
        let read_idx = datagram.read_idx.load(Ordering::Acquire);
        let current_num_msgs = datagram.current_num_msgs.load(Ordering::Acquire);

        // Check if queue is empty
        if current_num_msgs == 0 {
            return 0;
        }

        let next_read_idx = (read_idx + 1) % datagram.msg_capacity;
        datagram.read_idx.store(next_read_idx, Ordering::Release);

        datagram.current_num_msgs.fetch_sub(1, Ordering::AcqRel);

        let message = &datagram.messages[read_idx];

        let msg_data = message.get_data();
        let len = data.len().min(msg_data.len());
        data[..len].copy_from_slice(&msg_data[..len]);

        len
    }
}

#[derive(Debug)]
pub struct QueuingDestination(MmapMut);

impl TryFrom<RawFd> for QueuingDestination {
    type Error = TypedError;

    fn try_from(file: RawFd) -> Result<Self, Self::Error> {
        let mmap = unsafe { MmapMut::map_mut(file).typ(SystemError::Panic)? };

        Ok(Self(mmap))
    }
}

mod datagram_swap_tests {
    use std::sync::atomic::AtomicUsize;

    use itertools::Itertools;

    use crate::queuing::{Datagram, Message, Queuing};

    // TODO these tests do not even make sense, we need separate data buffers for source, destination and their expected contents

    /// No changes, destination and source buffer data is equal
    ///
    /// # Source
    /// | Messages                 | X | X | X | - |
    /// |-------------------------:|:-:|:-:|:-:|:-:|
    /// | read_idx                 | - | X | - | - |
    /// | write_idx                | - | - | - | X |
    ///
    /// # Destination
    /// | Messages                 | - | X | X | - |
    /// |-------------------------:|:-:|:-:|:-:|:-:|
    /// | read_idx                 | - | X | - | - |
    /// | write_idx                | - | - | - | X |
    #[test]
    fn no_changes() {
        test_swap(0, 0, 0, 0, &[&[], &[], &[], &[]], &[&[], &[], &[], &[]], 4);
    }

    /// The destination has read a single message
    ///
    /// # Source
    /// | Messages                 | X | X   | X     | - |
    /// |-------------------------:|:-:|:---:|:-----:|:-:|
    /// | data                     | 1 | 2,3 | 4,5,6 | - |
    /// | read_idx                 | X | -   | -     | - |
    /// | write_idx                | - | -   | -     | X |
    ///
    /// # Destination
    /// | Messages                 | - | X | X | - |
    /// |-------------------------:|:-:|:-:|:-:|:-:|
    /// | read_idx                 | - | X | - | - |
    /// | write_idx                | - | - | - | X |
    #[test]
    fn read_advanced() {
        test_swap(
            0,
            3,
            1,
            2,
            &[&[1], &[2, 3], &[4, 5, 6], &[7, 8, 9, 10]],
            &[&[1], &[2, 3], &[4, 5, 6], &[7, 8, 9, 10]],
            4,
        );
    }

    fn test_swap(
        source_read_idx: usize,
        source_num_msgs: usize,
        destination_read_idx: usize,
        destination_num_msgs: usize,
        initial_msg_data: &[&[u8]],
        expected_msg_data: &[&[u8]],
        msg_size: usize,
    ) {
        let source_read_idx = AtomicUsize::new(source_read_idx);
        let source_num_msgs = AtomicUsize::new(source_num_msgs);

        let destination_read_idx = AtomicUsize::new(destination_read_idx);
        let destination_num_msgs = AtomicUsize::new(destination_num_msgs);

        let mut message_lengths = initial_msg_data.iter().map(|x| x.len()).collect_vec();
        assert!(*message_lengths.iter().max().unwrap() <= msg_size);

        let mut message_data = initial_msg_data
            .iter()
            .flat_map(|msg| msg.iter().copied().pad_using(msg_size, |_| 0))
            .collect_vec();

        unsafe {
            let (source_datagram, destination_datagram) = initialize_swap_testing(
                &source_read_idx,
                &source_num_msgs,
                &destination_read_idx,
                &destination_num_msgs,
                message_lengths.as_mut_slice(),
                message_data.as_mut_slice(),
                msg_size,
            );

            Queuing::perform_swap(source_datagram, destination_datagram);
        }

        let expected_message_data = expected_msg_data
            .iter()
            .flat_map(|msg| msg.iter().copied().pad_using(msg_size, |_| 0))
            .collect_vec();

        assert_eq!(message_data, expected_message_data);
    }

    /// This function makes msg_data be shared mutably by both returned datagrams.
    /// Thus the returned datagrams may not be used to access msg_data simultaneously.
    unsafe fn initialize_swap_testing<'a>(
        source_read_idx: &'a AtomicUsize,
        source_num_msgs: &'a AtomicUsize,
        destination_read_idx: &'a AtomicUsize,
        destination_num_msgs: &'a AtomicUsize,
        msg_lengths: &'a mut [usize],
        msg_data: &'a mut [u8],
        msg_size: usize,
    ) -> (Datagram<'a>, Datagram<'a>) {
        assert_eq!(msg_lengths.len() * msg_size, msg_data.len());

        let mut messages = Vec::new();
        let mut remaining_msg_data: &mut [u8] = msg_data;
        for length in msg_lengths {
            let (data, rest) = remaining_msg_data.split_at_mut(4);
            remaining_msg_data = rest;
            messages.push(Message { len: length, data })
        }

        let source = Datagram {
            msg_size: 4,
            msg_capacity: 4,
            read_idx: source_read_idx,
            current_num_msgs: source_num_msgs,
            messages: messages
                .iter()
                .map(|x| unsafe { std::mem::transmute_copy(x) })
                .collect_vec(), // Here we do a little trolling 💀
        };

        let destination = Datagram {
            msg_size: 4,
            msg_capacity: 4,
            read_idx: destination_read_idx,
            current_num_msgs: destination_num_msgs,
            messages,
        };

        (source, destination)
    }
}
