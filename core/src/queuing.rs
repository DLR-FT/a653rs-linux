use std::fmt::{Debug, Formatter};
use std::mem::size_of;
use std::os::fd::RawFd;
use std::os::fd::{AsRawFd, OwnedFd};
use std::ptr::slice_from_raw_parts;
use std::slice::SliceIndex;

use a653rs::bindings::PortDirection;
use anyhow::bail;
use memfd::{FileSeal, Memfd, MemfdOptions};
use memmap2::MmapMut;

use crate::channel::{PortConfig, QueuingChannelConfig};
use crate::error::{ResultExt, SystemError, TypedError, TypedResult};
use crate::partition::QueuingConstant;

/// This is a reference to a ring buffer containing byte data living somewhere else im memory.
/// It will not use any additional memory and thus not allocate or free memory.
/// For usage one can initialize a ring buffer inside a provided buffer through [RingBufferRef::init_at].
/// Using [RingBufferRef::load] one may then create a reference to this ring buffer and access it.
struct RingBufferRef<'a> {
    entry_size: usize,
    capacity: usize,

    len: &'a mut usize,
    first: &'a mut usize, // idx of first entry in data
    data: &'a mut [u8],   // is of size `capacity * entry_size`
}

impl<'a> RingBufferRef<'a> {
    pub fn size(entry_size: usize, capacity: usize) -> usize {
        use std::mem::size_of;
        size_of::<usize>() // entry_size
            + size_of::<usize>() // capacity
            + size_of::<usize>() // len
            + size_of::<usize>() // first
            + entry_size * capacity // data
    }

    /// Takes a FnMut that creates a buffer object of type `B` with a provided size.
    /// The RingBuffer will calculate its required size, call the FnMut and then initialize itself in the buffers raw data.
    pub fn init_at(entry_size: usize, capacity: usize, buffer: &'a mut [u8]) -> Self {
        let required_size = Self::size(entry_size, capacity);
        assert_eq!(buffer.len(), required_size);

        let (entry_size_field, capacity_field, len_field, first_field, data_field) =
            unsafe { Self::load_fields(buffer) };

        *entry_size_field = entry_size;
        *capacity_field = capacity;
        *len_field = 0;
        *first_field = 0;

        Self {
            entry_size,
            capacity,
            len: len_field,
            first: first_field,
            data: data_field,
        }
    }

    unsafe fn load_fields(
        buffer: &mut [u8],
    ) -> (&mut usize, &mut usize, &mut usize, &mut usize, &mut [u8]) {
        let (entry_size, rest) = unsafe { buffer.strip_field_mut::<usize>() };
        let (capacity, rest) = unsafe { rest.strip_field_mut::<usize>() };
        let (len, rest) = unsafe { rest.strip_field_mut::<usize>() };
        let (first, data) = unsafe { rest.strip_field_mut::<usize>() };

        (entry_size, capacity, len, first, data)
    }

    pub unsafe fn load_from(buffer: &'a mut [u8]) -> Self {
        let buffer_size = buffer.len();
        let (&mut entry_size, &mut capacity, len, first, data) = Self::load_fields(buffer);

        assert_eq!(Self::size(entry_size, capacity), buffer_size);

        Self {
            entry_size,
            capacity,
            len,
            first,
            data,
        }
    }

    fn to_physical_idx(&self, idx: usize) -> usize {
        (*self.first + idx) % self.capacity * self.entry_size
    }

    pub fn get(&self, idx: usize) -> Option<&[u8]> {
        assert!(idx < self.capacity);

        (idx < *self.len).then(|| {
            let idx = self.to_physical_idx(idx);
            &self.data[idx..(idx + self.entry_size)]
        })
    }

    pub fn get_mut(&mut self, idx: usize) -> Option<&mut [u8]> {
        assert!(idx < self.capacity);

        (idx < *self.len).then(|| {
            let idx = self.to_physical_idx(idx);
            &mut self.data[idx..(idx + self.entry_size)]
        })
    }

    pub fn push(&mut self, entry_data: &[u8]) -> anyhow::Result<&mut [u8]> {
        assert_eq!(entry_data.len(), self.entry_size);
        self.push_with(|entry| entry.copy_from_slice(entry_data))
    }

    pub fn push_with<F: FnOnce(&'_ mut [u8])>(&mut self, f: F) -> anyhow::Result<&mut [u8]> {
        if *self.len == self.capacity {
            bail!("ring buffer reached maximum capacity");
        }

        *self.len += 1;

        let entry = self
            .get_mut(*self.len - 1)
            .expect("entry to exist, as length was just incremented");

        f(entry);

        Ok(entry)
    }
    fn pop(&mut self) -> Option<Vec<u8>> {
        self.pop_with(|entry| Vec::from(entry))
    }

    /// Pops the first entry and maps it with given closure.
    /// This is useful if the caller wants to prevent any allocations.
    /// Otherwise the [RingBufferRef::pop] function may be preferred.
    fn pop_with<F: FnOnce(&'_ [u8]) -> T, T>(&'_ mut self, mapper: F) -> Option<T> {
        let ret = self.get(0).map(mapper);

        if ret.is_some() {
            *self.first += 1;
            *self.first %= self.capacity;

            *self.len -= 1;
        }

        ret
    }
}

impl Debug for RingBufferRef<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RingBufferRef")
            .field("entry_size", &self.entry_size)
            .field("capacity", &self.capacity)
            .field("len", &self.len)
            .field("first", &self.first)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct SourceDatagram<'a> {
    num_messages_in_destination: &'a mut usize,
    ring_buffer: RingBufferRef<'a>,
}

#[derive(Debug)]
struct DestinationDatagram<'a> {
    ring_buffer: RingBufferRef<'a>,
}

impl<'a> SourceDatagram<'a> {
    fn size(msg_size: usize, msg_capacity: usize) -> usize {
        size_of::<usize>() // number of messages in destination
            + RingBufferRef::size(Message::size(msg_size), msg_capacity) // the ring buffer
    }

    fn init_at(msg_size: usize, msg_capacity: usize, buffer: &'a mut [u8]) -> Self {
        let (num_messages_in_destination, buffer) = unsafe { buffer.strip_field_mut::<usize>() };

        let ring_buffer = RingBufferRef::init_at(Message::size(msg_size), msg_capacity, buffer);

        Self {
            num_messages_in_destination,
            ring_buffer,
        }
    }

    unsafe fn load_from(buffer: &'a mut [u8]) -> Self {
        let (num_messages_in_destination, buffer) = unsafe { buffer.strip_field_mut::<usize>() };

        let ring_buffer = RingBufferRef::load_from(buffer);

        Self {
            num_messages_in_destination,
            ring_buffer,
        }
    }

    fn pop_with<F: FnOnce(Message<'_>) -> T, T>(&'_ mut self, f: F) -> Option<T> {
        self.ring_buffer
            .pop_with(|entry| f(Message::from_bytes(entry)))
    }

    fn push<'b>(&'b mut self, data: &'_ [u8]) -> anyhow::Result<Message<'b>> {
        if *self.num_messages_in_destination + *self.ring_buffer.len == self.ring_buffer.capacity {
            bail!("Failed to push message to source datagram. Queueing port is already at full capacity");
        }

        let entry = self
            .ring_buffer
            .push_with(|entry| Message::init_at(entry, data))?;

        Ok(Message::from_bytes(entry))
    }
}

impl<'a> DestinationDatagram<'a> {
    fn size(msg_size: usize, msg_capacity: usize) -> usize {
        RingBufferRef::size(Message::size(msg_size), msg_capacity) // the ring buffer
    }
    fn init_at(msg_size: usize, msg_capacity: usize, buffer: &'a mut [u8]) -> Self {
        Self {
            ring_buffer: RingBufferRef::init_at(Message::size(msg_size), msg_capacity, buffer),
        }
    }
    unsafe fn load_from(buffer: &'a mut [u8]) -> Self {
        Self {
            ring_buffer: RingBufferRef::load_from(buffer),
        }
    }

    fn pop_map<F: FnOnce(Message<'_>) -> T, T>(&mut self, msg_mapper: F) -> Option<T> {
        self.ring_buffer
            .pop_with(|entry| msg_mapper(Message::from_bytes(entry)))
    }

    fn push<'b>(&'b mut self, data: &'_ [u8]) -> anyhow::Result<Message<'b>> {
        let entry = self.ring_buffer.push(data)?;
        let msg = Message::from_bytes(entry);

        Ok(msg)
    }
}

struct Message<'a> {
    len: &'a usize,
    data: &'a [u8],
}

impl<'a> Message<'a> {
    fn size(msg_size: usize) -> usize {
        size_of::<usize>() // length of this message
            + msg_size // actual message byte data
    }
    fn from_bytes(bytes: &'a [u8]) -> Self {
        let (len, data) = unsafe { bytes.strip_field::<usize>() };
        assert!(*len <= data.len());

        Self { len, data }
    }

    fn init_at(uninitialized_bytes: &mut [u8], data: &[u8]) {
        let (len_field, data_field) = unsafe { uninitialized_bytes.strip_field_mut::<usize>() };
        assert!(data_field.len() >= data.len());

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

        let mut num_msg_swapped = 0;
        while let Some(_new_destination_msg) =
            source_datagram.pop_with(|msg| destination_datagram.push(msg.to_bytes()).expect("push to always succeed, because source and destination datagrams can only contain `msg_capacity` messages in total"))
        {
            num_msg_swapped += 1;
        }

        *source_datagram.num_messages_in_destination = *destination_datagram.ring_buffer.len;

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
        let mut datagram = unsafe { SourceDatagram::load_from(&mut self.0) };

        datagram.push(data).map(|m| *m.len).unwrap_or(0)
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
    pub fn read(&mut self, buffer: &mut [u8]) -> usize {
        let mut datagram = unsafe { DestinationDatagram::load_from(&mut self.0) };

        datagram
            .pop_map(|msg| {
                let data = msg.get_data();
                let len = data.len().min(buffer.len());
                buffer[..len].copy_from_slice(&data[..len]);

                len
            })
            .unwrap_or(0)
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
