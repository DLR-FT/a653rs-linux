use std::fmt::Debug;
use std::mem;
use std::mem::size_of;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::time::Instant;

use a653rs::bindings::PortDirection;
use datagrams::{DestinationDatagram, SourceDatagram};
use memfd::{FileSeal, Memfd, MemfdOptions};
use memmap2::MmapMut;
use message::Message;

use crate::channel::{PortConfig, QueuingChannelConfig};
use crate::error::{ResultExt, SystemError, TypedError, TypedResult};
use crate::partition::QueuingConstant;

mod datagrams;
mod message;
mod queue;

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
