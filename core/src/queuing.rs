use std::fmt::{Debug, Formatter};
use std::os::fd::RawFd;
use std::os::fd::{AsRawFd, OwnedFd};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use a653rs::bindings::PortDirection;
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
/// - `AtomicBool`: Flag that signals if source port contains unswapped changes
/// - List of messages:
///   - `usize`: Data size of this message
///   - `[u8; msg_size]`: Data of this message
struct Datagram<'m> {
    msg_size: usize,
    msg_capacity: usize,
    read_idx: &'m AtomicUsize,
    current_num_msgs: &'m AtomicUsize,
    /// A flag that signals whether new messages were written into a source port since its last swap.
    /// This is needed for recovering from a full source port queue.
    /// Note: Even though it is only relevant for source port, it should also be properly initialized for the destination port, just to prevent any UB when accessing it via this reference.
    has_unswapped_changes: &'m AtomicBool,
    /// As of now mutable references are stored
    /// Possible improvement: Maybe each message can be wrapped into its own [Mutex]. This would sacrifice some performance and memory for a little more safety when multiple processes are working on the same memory.
    messages: Vec<Message<'m>>,
}

impl<'m> Datagram<'m> {}

struct Message<'m> {
    len: &'m mut usize,
    data: &'m mut [u8], // this slice is actually of length msg_size which will be equal for all messages in a queue
}

impl Debug for Datagram<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Datagram")
            .field("msg_size", &self.msg_size)
            .field("msg_capacity", &self.msg_capacity)
            .field("read_idx", &self.read_idx)
            .field("current_num_msgs", &self.current_num_msgs)
            .finish_non_exhaustive()
    }
}

impl Debug for Message<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Message")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
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
        + size_of::<AtomicUsize>() // Flag if this datagram has unswapped changes
        + msg_num * (size_of::<usize>() + msg_size) // Message length header and contents
    }

    fn init_at(mmap: &mut MmapMut, msg_size: usize, queue_capacity: usize) {
        let mmap_bytes = (*mmap).as_mut();

        // Split mmap bytes slice into the contained fields
        let (msg_size_ref, rest) = unsafe { Self::strip_field_mut::<usize>(mmap_bytes) };
        let (queue_capacity_ref, rest) = unsafe { Self::strip_field_mut::<usize>(rest) };
        let (read_idx, rest) = unsafe { Self::strip_field_mut::<AtomicUsize>(rest) };
        let (current_num_msgs, rest) = unsafe { Self::strip_field_mut::<AtomicUsize>(rest) };
        let (has_unswapped_changes, _) = unsafe { Self::strip_field_mut::<AtomicBool>(rest) };

        *msg_size_ref = msg_size;
        *queue_capacity_ref = queue_capacity;

        unsafe {
            ptr::write(read_idx as *mut AtomicUsize, AtomicUsize::new(0));
        }
        unsafe {
            ptr::write(current_num_msgs as *mut AtomicUsize, AtomicUsize::new(0));
        }
        unsafe {
            ptr::write(
                has_unswapped_changes as *mut AtomicBool,
                AtomicBool::new(true),
            );
        }
    }

    /// # Safety
    /// It is required for `bytes` to contain a T at the beginning
    unsafe fn strip_field_mut<T>(bytes: &mut [u8]) -> (&mut T, &mut [u8]) {
        debug_assert!(bytes.len() >= std::mem::size_of::<T>());
        let (field, rest) = bytes.split_at_mut(std::mem::size_of::<T>());
        let field = (field.as_ptr() as *mut T).as_mut().unwrap();
        (field, rest)
    }

    /// # Safety
    /// It is required for the mapped memory to be initialized with [Datagram::init_at]
    unsafe fn read(mmap: &mut MmapMut) -> Datagram {
        // Get the bytes from the mapped memory
        let mmap_bytes = (*mmap).as_mut();

        // Split mmap bytes slice into the contained fields
        let (&mut msg_size, rest) = unsafe { Self::strip_field_mut::<usize>(mmap_bytes) };
        let (&mut queue_capacity, rest) = unsafe { Self::strip_field_mut::<usize>(rest) };
        let (read_idx, rest) = unsafe { Self::strip_field_mut::<AtomicUsize>(rest) };
        let (current_num_msgs, rest) = unsafe { Self::strip_field_mut::<AtomicUsize>(rest) };
        let (has_unswapped_changes, mut msg_data) =
            unsafe { Self::strip_field_mut::<AtomicBool>(rest) };

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
            has_unswapped_changes,
        }
    }

    fn clear(&self) {
        self.current_num_msgs.store(0, Ordering::Release);
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

    /// Returns number of copied messages
    fn perform_swap(source: &Datagram, destination: &mut Datagram) -> usize {
        // First get some necessary values for both source and destination
        debug_assert_eq!(source.msg_capacity, destination.msg_capacity);
        let capacity = source.msg_capacity;

        // Then calculate how many messages were read by the destination...
        let num_msgs_read = (destination.read_idx.load(Ordering::Acquire) + capacity
            - source.read_idx.load(Ordering::Acquire))
            % capacity;
        // ...and how many were written by the source
        let num_msgs_written = (source.current_num_msgs.load(Ordering::Acquire)
            - destination.current_num_msgs.load(Ordering::Acquire)
            - num_msgs_read);

        // Now we can
        // # 1. Advance source by number of read messages / Data transfer from destination to source
        // - Calculate num msgs read by destination through difference of read indices
        // - Copy read idx from destination to source
        // - Subtract num msgs read from source.num

        // # 2. Copy messages / Data transfer from source to destination
        // - Copy messages from dest.read_idx+dest.num until source.read_idx+source.num
        // - Increment dest.num

        // 1.
        let read_idx_destination = destination.read_idx.load(Ordering::Acquire);

        source
            .read_idx
            .store(read_idx_destination, Ordering::Release);
        source
            .current_num_msgs
            .fetch_sub(num_msgs_read, Ordering::AcqRel);

        // 2.
        let num_msgs_in_destination = destination.current_num_msgs.load(Ordering::Acquire);
        let start_new_msgs = (read_idx_destination + num_msgs_in_destination) % capacity;

        let new_messages = (start_new_msgs..(start_new_msgs + num_msgs_written + capacity))
            .map(|i| i % source.msg_capacity);
        new_messages.for_each(|msg_idx| {
            let src_msg = &source.messages[msg_idx];
            let mut destination_msg = &mut destination.messages[msg_idx];

            destination_msg.copy_from_msg(src_msg)
        });

        destination
            .current_num_msgs
            .fetch_add(num_msgs_written, Ordering::Release);

        num_msgs_written
    }

    /// Returns true if messages have been transferred
    pub fn swap(&mut self) -> bool {
        let source_datagram = unsafe { Datagram::read(&mut self.source_receiver) };

        // Parse destination datagram
        let mut destination_datagram = unsafe { Datagram::read(&mut self.destination_sender) };

        // If the source does not contain any unswapped messages and there are no messages left in the destination,
        // we can assume that all messages (those residing in source) are already swapped and read.
        // Thus we can must clear the source to make room for new messages.
        if !source_datagram
            .has_unswapped_changes
            .load(Ordering::Acquire)
            && source_datagram.current_num_msgs.load(Ordering::Acquire)
                == source_datagram.msg_capacity
            && destination_datagram
                .current_num_msgs
                .load(Ordering::Acquire)
                == 0
        {
            source_datagram.clear();
        }

        let num_msg_swapped = Self::perform_swap(&source_datagram, &mut destination_datagram);

        if num_msg_swapped == source_datagram.current_num_msgs.load(Ordering::Acquire) {
            source_datagram
                .has_unswapped_changes
                .store(false, Ordering::Release);
        }

        trace!("Swapped {num_msg_swapped} messages: Destination={destination_datagram:?} Source={source_datagram:?}");

        num_msg_swapped > 0
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
        let mut datagram = unsafe { Datagram::read(&mut self.0) };
        let read_idx = datagram.read_idx.load(Ordering::Acquire);
        let current_num_msgs = datagram.current_num_msgs.load(Ordering::Acquire);

        // Check if queue is full
        if current_num_msgs == datagram.msg_capacity {
            return 0;
        }

        datagram.current_num_msgs.fetch_add(1, Ordering::Release);
        datagram
            .has_unswapped_changes
            .store(true, Ordering::Release);

        let write_idx = (read_idx + current_num_msgs) % datagram.msg_capacity;

        datagram.messages[write_idx].set_data(data)
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
        let datagram = unsafe { Datagram::read(&mut self.0) };
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
