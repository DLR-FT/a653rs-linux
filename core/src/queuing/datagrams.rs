use std::fmt::Debug;
use std::mem::size_of;
use std::time::Instant;

use crate::queuing::message::Message;
use crate::queuing::queue::ConcurrentQueue;
use crate::queuing::StripFieldExt;

#[derive(Debug)]
pub struct SourceDatagram<'a> {
    pub num_messages_in_destination: &'a mut usize,
    pub has_overflowed: &'a mut bool,
    pub message_queue: &'a ConcurrentQueue,
}

#[derive(Debug)]
pub struct DestinationDatagram<'a> {
    pub num_messages_in_source: &'a mut usize,
    pub clear_requested_timestamp: &'a mut Option<Instant>,
    pub has_overflowed: &'a mut bool,
    pub message_queue: &'a ConcurrentQueue,
}

impl<'a> SourceDatagram<'a> {
    pub fn size(msg_size: usize, msg_capacity: usize) -> usize {
        size_of::<usize>() // number of messages in destination
            + size_of::<bool>() // flag if queue has overflowed
            + ConcurrentQueue::size(Message::size(msg_size), msg_capacity) // the message queue
    }

    pub fn init_at(msg_size: usize, msg_capacity: usize, buffer: &'a mut [u8]) -> Self {
        let (num_messages_in_destination, buffer) = unsafe { buffer.strip_field_mut::<usize>() };
        let (has_overflowed, buffer) = unsafe { buffer.strip_field_mut::<bool>() };

        let message_queue = ConcurrentQueue::init_at(buffer, Message::size(msg_size), msg_capacity);

        Self {
            num_messages_in_destination,
            has_overflowed,
            message_queue,
        }
    }

    pub unsafe fn load_from(buffer: &'a mut [u8]) -> Self {
        let (num_messages_in_destination, buffer) = unsafe { buffer.strip_field_mut::<usize>() };
        let (has_overflowed, buffer) = unsafe { buffer.strip_field_mut::<bool>() };

        let message_queue = ConcurrentQueue::load_from(buffer);

        Self {
            num_messages_in_destination,
            has_overflowed,
            message_queue,
        }
    }

    pub fn pop_then<F: FnOnce(Message<'_>) -> T, T>(&'_ mut self, f: F) -> Option<T> {
        self.message_queue
            .pop_then(|entry| f(Message::from_bytes(entry)))
    }

    pub fn push<'b>(
        &'b mut self,
        data: &'_ [u8],
        message_timestamp: Instant,
    ) -> Option<Message<'b>> {
        // We need to check if there is enough space left in the queue.
        // This is important, because we could theoretically store twice the number of
        // our queue size, because we use a separate source and destination queueu.
        // Thus we need to limit the number of messages in both queues at the same time.
        let queue_is_full = *self.num_messages_in_destination + self.message_queue.len()
            == self.message_queue.msg_capacity;

        if queue_is_full {
            *self.has_overflowed = true;
            return None;
        } else {
            *self.has_overflowed = false;
        }
        let entry = self.message_queue
            .push_then(|entry| Message::init_at(entry, data, message_timestamp)).expect("push to be successful because we just checked if there is space in both the source and destination");

        Some(Message::from_bytes(entry))
    }
}

impl<'a> DestinationDatagram<'a> {
    pub fn size(msg_size: usize, msg_capacity: usize) -> usize {
        size_of::<usize>() // number of messages in source
            + size_of::<bool>() // flag if queue is overflowed
            + size_of::<Option<Instant>>() // flag for the timestamp when a clear was requested
            + ConcurrentQueue::size(Message::size(msg_size), msg_capacity) // the message queue
    }
    pub fn init_at(msg_size: usize, msg_capacity: usize, buffer: &'a mut [u8]) -> Self {
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
    pub unsafe fn load_from(buffer: &'a mut [u8]) -> Self {
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
    pub fn pop_then<F: FnOnce(Message<'_>) -> T, T>(&mut self, msg_mapper: F) -> Option<(T, bool)> {
        self.message_queue
            .pop_then(|entry| msg_mapper(Message::from_bytes(entry)))
            .map(|t| (t, *self.has_overflowed))
    }

    /// Pushes a data onto the destination queue
    pub fn push<'b>(&'b mut self, data: &'_ [u8]) -> Option<Message<'b>> {
        let entry = self.message_queue.push(data)?;
        let msg = Message::from_bytes(entry);

        Some(msg)
    }
}
