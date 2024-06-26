use std::fmt::Debug;
use std::mem;
use std::mem::size_of;
use std::os::fd::RawFd;
use std::os::fd::{AsRawFd, OwnedFd};
use std::ptr::slice_from_raw_parts;
use std::time::Instant;

use a653rs::bindings::PortDirection;
use memfd::{FileSeal, Memfd, MemfdOptions};
use memmap2::MmapMut;

use crate::channel::{PortConfig, QueuingChannelConfig};
use crate::error::{ResultExt, SystemError, TypedError, TypedResult};
use crate::partition::QueuingConstant;
use crate::queuing::concurrent_queue::ConcurrentQueue;

pub mod concurrent_queue {
    use std::cell::UnsafeCell;
    use std::fmt::{Debug, Formatter};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::{mem, ptr};

    /// An unsized bounded concurrent queue (Fifo) that makes use of atomics and
    /// does not use pointers internally. This allows the queue to be
    /// created inside a buffer of type `&[u8]` via [ConcurrentQueue::init_at].
    /// The required buffer size can be requested in advance via
    /// [ConcurrentQueue::size] by providing the size and maximum number of
    /// entries. # Example
    /// ```
    /// # use a653rs_linux_core::queuing::concurrent_queue::ConcurrentQueue;
    /// // Create a ConcurrentQueue inside of a Vec<u8> buffer object
    /// let required_size = ConcurrentQueue::size(1, 4);
    /// let mut  buffer = vec![0u8; required_size];
    /// ConcurrentQueue::init_at(&mut buffer, 1, 4);
    /// let queue1 = unsafe { ConcurrentQueue::load_from(&buffer) };
    /// let queue2 = unsafe { ConcurrentQueue::load_from(&buffer) };
    ///
    /// // Let's push some values in the queue
    /// assert!(queue1.push(&[1]).is_some());
    /// assert!(queue2.push(&[2]).is_some());
    ///
    /// // Now pop them using the Fifo method
    /// assert_eq!(queue2.pop().unwrap()[0], 1);
    /// assert_eq!(queue1.pop().unwrap()[0], 2);
    ///
    /// // When the queue is empty, pop will return None
    /// assert_eq!(queue1.pop(), None);
    /// assert_eq!(queue2.pop(), None);
    /// ```
    #[repr(C)]
    pub struct ConcurrentQueue {
        pub msg_size: usize,
        pub msg_capacity: usize,

        len: AtomicUsize,
        first: AtomicUsize,
        data: UnsafeCell<[u8]>,
    }

    unsafe impl Send for ConcurrentQueue {}
    unsafe impl Sync for ConcurrentQueue {}

    impl ptr_meta::Pointee for ConcurrentQueue {
        type Metadata = usize;
    }

    impl ConcurrentQueue {
        /// Calculates the required buffer size to fit a MessageQueue object
        /// with `capacity` maximum elements and a fixed size of `element_size`
        /// bytes per element.
        pub fn size(element_size: usize, capacity: usize) -> usize {
            let mut size = Self::fields_size() + element_size * capacity; // data

            // We need to include extra padding for calculating this structs size,
            // because of `#[repr(C)]` the compiler may add padding to this struct for
            // alignment purposes,
            let alignment = Self::align();
            let sub_alignment_mask = alignment - 1;
            if size & sub_alignment_mask > 0 {
                // If the size ended with non-aligned bytes, we add the necessary padding.
                size = (size & !sub_alignment_mask) + alignment;
            }

            size
        }

        /// Returns the size of bytes of this struct's fields
        fn fields_size() -> usize {
            mem::size_of::<usize>() // entry_size
                + mem::size_of::<usize>() // capacity
                + mem::size_of::<AtomicUsize>() // len
                + mem::size_of::<AtomicUsize>() // first
        }

        /// Returns this struct's alignment
        fn align() -> usize {
            // This structs maximum alignment is that of a usize (or AtomicUsize, which has
            // the same data layout)
            mem::align_of::<usize>()
        }

        /// Creates a new empty ConcurrentQueue in given buffer.
        /// Even though this function returns a reference to the newly created
        /// ConcurrentQueue, it should be dropped to release the mutable
        /// reference to the buffer.
        ///
        /// # Panics
        /// If the buffer size is not exactly the required size to fit this
        /// `ConcurrentQueue` object.
        pub fn init_at(buffer: &mut [u8], element_size: usize, capacity: usize) -> &Self {
            assert_eq!(buffer.len(), Self::size(element_size, capacity));

            // We cast the `buffer` reference to a `Self` pointer, which can then safely be
            // dereferenced
            let queue = unsafe { &mut *Self::buf_to_self_mut(buffer) };

            queue.msg_size = element_size;
            queue.msg_capacity = capacity;
            // Use `ptr::write` to prevent the compiler from trying to drop previous values.
            unsafe {
                ptr::write(&mut queue.len, AtomicUsize::new(0));
                ptr::write(&mut queue.first, AtomicUsize::new(0));
            }

            queue
        }

        /// Converts the given buffer pointer to a ConcurrentQueue pointer and
        /// handles shortening the wide-pointer metadata.
        fn buf_to_self(buffer: *const [u8]) -> *const Self {
            let (buf_ptr, mut buf_len): (*const (), usize) = ptr_meta::PtrExt::to_raw_parts(buffer);
            buf_len -= Self::fields_size();

            ptr_meta::from_raw_parts(buf_ptr, buf_len)
        }

        /// Converts the given mutable buffer pointer to a ConcurrentQueue
        /// pointer and handles shortening the wide-pointer metadata.
        fn buf_to_self_mut(buffer: *mut [u8]) -> *mut Self {
            let (buf_ptr, mut buf_len): (*mut (), usize) = ptr_meta::PtrExt::to_raw_parts(buffer);
            buf_len -= Self::fields_size();

            ptr_meta::from_raw_parts_mut(buf_ptr, buf_len)
        }

        /// Loads a `ConcurrentQueue` from the specified buffer.
        /// # Safety
        /// The buffer must contain exactly one valid ConcurrentQueue, which has
        /// to be initialized through [ConcurrentQueue::init_at]. Also
        /// mutating or reading raw values from the buffer may result in
        /// UB, because the ConcurrentQueue relies on internal safety mechanisms
        /// to prevent UB due to shared mutable state.
        pub unsafe fn load_from(buffer: &[u8]) -> &Self {
            let obj = &*Self::buf_to_self(buffer);

            // Perform some validity checks
            debug_assert!(obj.len.load(Ordering::SeqCst) <= obj.msg_capacity); // Check length
            debug_assert!(obj.first.load(Ordering::SeqCst) < obj.msg_capacity); // Check first idx

            // Also check if unsized data field is of correct size
            // Note: obj_data may be longer than `obj.msg_size * obj.msg_capacity` due to
            // alignment padding. To correct we call `Self::size`.
            let obj_data = obj.data.get().as_ref().unwrap();
            debug_assert_eq!(
                obj_data.len(),
                Self::size(obj.msg_size, obj.msg_capacity) - Self::fields_size()
            );

            obj
        }

        /// Calculates the physical starting index of an element inside of the
        /// data array.
        fn to_physical_idx(&self, first: usize, idx: usize) -> usize {
            (first + idx) % self.msg_capacity * self.msg_size
        }

        /// Gets an element from the queue at a specific index
        pub fn get(&self, idx: usize) -> Option<&[u8]> {
            assert!(idx < self.msg_capacity);

            let current_len = self.len.load(Ordering::SeqCst);
            if idx > current_len {
                return None;
            }

            let idx = self.to_physical_idx(self.first.load(Ordering::SeqCst), idx);

            let msg = &unsafe { self.data.get().as_mut().unwrap() }[idx..(idx + self.msg_size)];
            Some(msg)
        }

        /// Pushes an element to the back of the queue. If there was space, a
        /// mutable reference to the inserted element is returned.
        pub fn push(&self, data: &[u8]) -> Option<&mut [u8]> {
            assert_eq!(data.len(), self.msg_size);

            self.push_then(|entry| entry.copy_from_slice(data))
        }

        /// Pushes an uninitialized element and then calls a closure to set its
        /// memory in-place. If there was space, a mutable reference to
        /// the inserted element is returned.
        pub fn push_then<F: FnOnce(&'_ mut [u8])>(&self, set_element: F) -> Option<&mut [u8]> {
            let current_len = self.len.load(Ordering::SeqCst);
            if current_len == self.msg_capacity {
                return None;
            }

            let insert_idx = self.len.fetch_add(1, Ordering::SeqCst);

            let idx = self.to_physical_idx(self.first.load(Ordering::SeqCst), insert_idx);
            let element_slot =
                &mut unsafe { self.data.get().as_mut().unwrap() }[idx..(idx + self.msg_size)];

            set_element(element_slot);

            Some(element_slot)
        }

        /// Tries to pop an element from the front of the queue.
        pub fn pop(&self) -> Option<Box<[u8]>> {
            self.pop_then(|entry| Vec::from(entry).into_boxed_slice())
        }

        /// Calls a mapping closure on the first element that is about to be
        /// popped from the queue. Only the return value of the closure
        /// is returned by this function. If the popped element is
        /// needed as owned data, consider using [ConcurrentQueue::pop] instead.
        pub fn pop_then<F: FnOnce(&'_ [u8]) -> T, T>(&'_ self, map_element: F) -> Option<T> {
            // Decrement length
            self.len
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |len| len.checked_sub(1))
                .ok()?;

            let prev_first = self
                .first
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |x| {
                    Some((x + 1) % self.msg_capacity)
                })
                .unwrap();

            let idx = self.to_physical_idx(prev_first, 0);

            let msg = &unsafe { &*self.data.get() }[idx..(idx + self.msg_size)];

            Some(map_element(msg))
        }

        pub fn peek_then<T, F: FnOnce(Option<&[u8]>) -> T>(&self, f: F) -> T {
            let len = self.len.load(Ordering::SeqCst);

            let msg = (len > 0).then(|| {
                let first = self.first.load(Ordering::SeqCst);
                let idx = self.to_physical_idx(first, 0);
                unsafe { &(&*self.data.get())[idx..(idx + self.msg_size)] }
            });

            f(msg)
        }

        /// Returns the current length of this queue
        pub fn len(&self) -> usize {
            self.len.load(Ordering::SeqCst)
        }

        #[must_use]
        pub fn is_empty(&self) -> bool {
            self.len() == 0
        }

        pub fn clear(&self) {
            self.len.store(0, Ordering::SeqCst);
        }
    }

    impl Debug for ConcurrentQueue {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ConcurrentQueue")
                .field("msg_size", &self.msg_size)
                .field("msg_capacity", &self.msg_capacity)
                .field("len", &self.len)
                .field("first", &self.first)
                .finish_non_exhaustive()
        }
    }
}

#[derive(Debug)]
struct SourceDatagram<'a> {
    num_messages_in_destination: &'a mut usize,
    has_overflowed: &'a mut bool,
    message_queue: &'a ConcurrentQueue,
}

#[derive(Debug)]
struct DestinationDatagram<'a> {
    num_messages_in_source: &'a mut usize,
    clear_requested_timestamp: &'a mut Option<Instant>,
    has_overflowed: &'a mut bool,
    message_queue: &'a ConcurrentQueue,
}

impl<'a> SourceDatagram<'a> {
    fn size(msg_size: usize, msg_capacity: usize) -> usize {
        size_of::<usize>() // number of messages in destination
            + size_of::<bool>() // flag if queue has overflowed
            + ConcurrentQueue::size(Message::size(msg_size), msg_capacity) // the message queue
    }

    fn init_at(msg_size: usize, msg_capacity: usize, buffer: &'a mut [u8]) -> Self {
        let (num_messages_in_destination, buffer) = unsafe { buffer.strip_field_mut::<usize>() };
        let (has_overflowed, buffer) = unsafe { buffer.strip_field_mut::<bool>() };

        let message_queue = ConcurrentQueue::init_at(buffer, Message::size(msg_size), msg_capacity);

        Self {
            num_messages_in_destination,
            has_overflowed,
            message_queue,
        }
    }

    unsafe fn load_from(buffer: &'a mut [u8]) -> Self {
        let (num_messages_in_destination, buffer) = unsafe { buffer.strip_field_mut::<usize>() };
        let (has_overflowed, buffer) = unsafe { buffer.strip_field_mut::<bool>() };

        let message_queue = ConcurrentQueue::load_from(buffer);

        Self {
            num_messages_in_destination,
            has_overflowed,
            message_queue,
        }
    }

    fn pop_then<F: FnOnce(Message<'_>) -> T, T>(&'_ mut self, f: F) -> Option<T> {
        self.message_queue
            .pop_then(|entry| f(Message::from_bytes(entry)))
    }

    fn push<'b>(&'b mut self, data: &'_ [u8], message_timestamp: Instant) -> Option<Message<'b>> {
        // We need to check if there is enough space left in the queue.
        // This is important, because we could theoretically store twice the number of
        // our queue size, because we use a separate source and destination queueu.
        // Thus we need to limit the number of messages in both queues at the same time.
        let queue_is_full = *self.num_messages_in_destination + self.message_queue.len()
            == self.message_queue.msg_capacity;

        if queue_is_full {
            *self.has_overflowed = true;
            return None;
        }
        let entry = self.message_queue
            .push_then(|entry| Message::init_at(entry, data, message_timestamp)).expect("push to be successful because we just checked if there is space in both the source and destination");

        Some(Message::from_bytes(entry))
    }
}

impl<'a> DestinationDatagram<'a> {
    fn size(msg_size: usize, msg_capacity: usize) -> usize {
        size_of::<usize>() // number of messages in source
            + size_of::<bool>() // flag if queue is overflowed
            + size_of::<Option<Instant>>() // flag for the timestamp when a clear was requested
            + ConcurrentQueue::size(Message::size(msg_size), msg_capacity) // the message queue
    }
    fn init_at(msg_size: usize, msg_capacity: usize, buffer: &'a mut [u8]) -> Self {
        let (num_messages_in_source, buffer) = unsafe { buffer.strip_field_mut::<usize>() };
        let (clear_requested_timestamp, buffer) =
            unsafe { buffer.strip_field_mut::<Option<Instant>>() };
        let (has_overflowed, buffer) = unsafe { buffer.strip_field_mut::<bool>() };

        *num_messages_in_source = 0;
        unsafe {
            std::ptr::write(clear_requested_timestamp, None);
            std::ptr::write(has_overflowed, false);
        }

        Self {
            num_messages_in_source,
            clear_requested_timestamp,
            has_overflowed,
            message_queue: ConcurrentQueue::init_at(buffer, Message::size(msg_size), msg_capacity),
        }
    }
    unsafe fn load_from(buffer: &'a mut [u8]) -> Self {
        let (num_messages_in_source, buffer) = unsafe { buffer.strip_field_mut::<usize>() };
        let (clear_requested_timestamp, buffer) =
            unsafe { buffer.strip_field_mut::<Option<Instant>>() };
        let (has_overflown, buffer) = unsafe { buffer.strip_field_mut::<bool>() };

        Self {
            num_messages_in_source,
            clear_requested_timestamp,
            has_overflowed: has_overflown,
            message_queue: ConcurrentQueue::load_from(buffer),
        }
    }

    /// Takes a closure that maps the popped message to some type.
    /// If there is a message in the queue, the resulting type and a flag
    /// whether the queue has overflowed is returned.
    fn pop_then<F: FnOnce(Message<'_>) -> T, T>(&mut self, msg_mapper: F) -> Option<(T, bool)> {
        self.message_queue
            .pop_then(|entry| msg_mapper(Message::from_bytes(entry)))
            .map(|t| (t, *self.has_overflowed))
    }

    /// Pushes a data onto the destination queue
    fn push<'b>(&'b mut self, data: &'_ [u8]) -> Option<Message<'b>> {
        let entry = self.message_queue.push(data)?;
        let msg = Message::from_bytes(entry);

        Some(msg)
    }
}

struct Message<'a> {
    len: &'a usize,
    timestamp: &'a Instant,
    /// This data slice is always of the same size, controlled by the owning
    /// ConcurrentQueue. That means, that only the first `self.len` bytes in
    /// it contain actual data. Use [Message::get_data] to access just the
    /// contained bytes.
    data: &'a [u8],
}

impl<'a> Message<'a> {
    fn size(msg_size: usize) -> usize {
        size_of::<usize>() // length of this message
            + size_of::<Instant>() // timestamp when this message was sent
            + msg_size // actual message byte data
    }
    fn from_bytes(bytes: &'a [u8]) -> Self {
        let (len, bytes) = unsafe { bytes.strip_field::<usize>() };
        let (timestamp, data) = unsafe { bytes.strip_field::<Instant>() };

        assert!(
            *len <= data.len(),
            "*len={} data.len()={}",
            *len,
            data.len()
        );

        Self {
            len,
            timestamp,
            data,
        }
    }

    fn init_at(uninitialized_bytes: &mut [u8], data: &[u8], initialization_timestamp: Instant) {
        let (len_field, uninitialized_bytes) =
            unsafe { uninitialized_bytes.strip_field_mut::<usize>() };
        let (timestamp, data_field) = unsafe { uninitialized_bytes.strip_field_mut::<Instant>() };
        assert!(data_field.len() >= data.len());

        unsafe {
            std::ptr::write(timestamp, initialization_timestamp);
        }

        *len_field = data.len();
        data_field[0..data.len()].copy_from_slice(data);
    }

    fn to_bytes(&self) -> &[u8] {
        // # Safety
        // len and data should be contiguous memory
        unsafe {
            &*slice_from_raw_parts(
                self.len as *const usize as *const u8,
                Self::size(self.data.len()),
            )
        }
    }

    fn get_data(&self) -> &[u8] {
        &self.data[0..*self.len]
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

        Ok(Self {
            msg_size,
            max_num_msg: msg_num,
            source_receiver,
            source,
            source_port: config.source,
            destination_sender,
            destination,
            destination_port: config.destination,
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

    fn memfd(name: impl AsRef<str>, size: usize) -> TypedResult<Memfd> {
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
        let mem = Self::memfd(name, SourceDatagram::size(msg_size, max_num_msgs))?;

        let mut mmap = unsafe { MmapMut::map_mut(mem.as_raw_fd()).typ(SystemError::Panic)? };

        mem.add_seals(&[FileSeal::SealSeal])
            .typ(SystemError::Panic)?;

        SourceDatagram::init_at(msg_size, max_num_msgs, mmap.as_mut());

        Ok((mmap, mem.into_file().into()))
    }

    fn destination(
        name: impl AsRef<str>,
        msg_size: usize,
        msg_capacity: usize,
    ) -> TypedResult<(MmapMut, OwnedFd)> {
        let mem = Self::memfd(name, DestinationDatagram::size(msg_size, msg_capacity))?;

        let mut mmap = unsafe { MmapMut::map_mut(mem.as_raw_fd()).typ(SystemError::Panic)? };

        mem.add_seals(&[FileSeal::SealSeal])
            .typ(SystemError::Panic)?;

        DestinationDatagram::init_at(msg_size, msg_capacity, mmap.as_mut());

        Ok((mmap, mem.into_file().into()))
    }

    /// Returns true if messages have been transferred
    pub fn swap(&mut self) -> bool {
        // Parse datagrams
        let mut source_datagram =
            unsafe { SourceDatagram::load_from(self.source_receiver.as_mut()) };
        let mut destination_datagram =
            unsafe { DestinationDatagram::load_from(self.destination_sender.as_mut()) };

        // If a clear was requested by the destination, we pop all messages from the
        // source queue with a timestamp before the timestamp of the clear request.
        // This is not actually needed for ARINC653 Part 4, as only one partition can
        // run at a time and all messages are swapped to the destination buffer after
        // every partition execution.
        if let Some(clear_requested_at) = mem::take(destination_datagram.clear_requested_timestamp)
        {
            while source_datagram.message_queue.peek_then(|msg| {
                msg.map_or(false, |msg| {
                    &clear_requested_at > Message::from_bytes(msg).timestamp
                })
            }) {
                source_datagram.message_queue.pop_then(|_| ());
            }
        };

        // Copy new messages from source to destination
        let mut num_msg_swapped = 0;
        while let Some(_new_destination_msg) =
            source_datagram.pop_then(|msg| destination_datagram.push(msg.to_bytes()).expect("push to always succeed, because source and destination datagrams can only contain `msg_capacity` messages in total"))
        {
            num_msg_swapped += 1;
        }

        *source_datagram.num_messages_in_destination = destination_datagram.message_queue.len();
        *destination_datagram.has_overflowed = *source_datagram.has_overflowed;

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
    /// If the message was successfully enqueued, the number of bytes written is
    /// returned.
    pub fn write(&mut self, data: &[u8], message_timestamp: Instant) -> Option<usize> {
        let mut datagram = unsafe { SourceDatagram::load_from(&mut self.0) };

        let res = datagram.push(data, message_timestamp).map(|msg| *msg.len);

        if res.is_some() {
            // The standard states, that the receiver should only be able to detect whether
            // the last message caused an overflow. Because we have now sent a
            // message successfully, thus we can now reset this flag.
            *datagram.has_overflowed = false;
        }

        res
    }

    pub fn get_current_num_messages(&mut self) -> usize {
        let datagram = unsafe { SourceDatagram::load_from(&mut self.0) };

        datagram.message_queue.len() + *datagram.num_messages_in_destination
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
    /// Reads the current message from the queue into a buffer and increments
    /// the current read index. If a message was successfully read, the
    /// number of bytes read and whether the queue has overflowed.
    pub fn read(&mut self, buffer: &mut [u8]) -> Option<(usize, bool)> {
        let mut datagram = unsafe { DestinationDatagram::load_from(&mut self.0) };

        let read_bytes_and_overflowed_flag = datagram.pop_then(|msg| {
            let data = msg.get_data();
            let len = data.len().min(buffer.len());
            buffer[..len].copy_from_slice(&data[..len]);

            len
        });

        read_bytes_and_overflowed_flag
    }

    pub fn get_current_num_messages(&mut self) -> usize {
        let datagram = unsafe { DestinationDatagram::load_from(&mut self.0) };
        datagram.message_queue.len() + *datagram.num_messages_in_source
    }

    pub fn clear(&mut self, current_time: Instant) {
        let datagram = unsafe { DestinationDatagram::load_from(&mut self.0) };
        datagram.message_queue.clear();
        *datagram.clear_requested_timestamp = Some(current_time);
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

/// An extension trait for stripping generic types off of byte arrays.
trait StripFieldExt {
    unsafe fn strip_field<T>(&self) -> (&T, &Self);
    unsafe fn strip_field_mut<T>(&mut self) -> (&mut T, &mut Self);
}

impl StripFieldExt for [u8] {
    /// # Safety
    /// The byte array must start with an initialized and valid `T`
    unsafe fn strip_field<T>(&self) -> (&T, &Self) {
        assert!(self.len() >= size_of::<T>());
        let (field, rest) = self.split_at(size_of::<T>());
        let field = (field.as_ptr() as *const T).as_ref().unwrap();
        (field, rest)
    }

    /// The byte array must start with an initialized and valid `T`
    unsafe fn strip_field_mut<T>(&mut self) -> (&mut T, &mut Self) {
        assert!(self.len() >= size_of::<T>());
        let (field, rest) = self.split_at_mut(size_of::<T>());
        let field = (field.as_ptr() as *mut T).as_mut().unwrap();
        (field, rest)
    }
}
