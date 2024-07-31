use std::mem::size_of;
use std::ptr::slice_from_raw_parts;
use std::time::Instant;

use super::StripFieldExt;

pub struct Message<'a> {
    pub len: &'a usize,
    pub timestamp: &'a Instant,
    /// This data slice is always of the same size, controlled by the owning
    /// ConcurrentQueue. That means, that only the first `self.len` bytes in
    /// it contain actual data. Use [Message::get_data] to access just the
    /// contained bytes.
    pub data: &'a [u8],
}

impl<'a> Message<'a> {
    pub fn size(msg_size: usize) -> usize {
        size_of::<usize>() // length of this message
            + size_of::<Instant>() // timestamp when this message was sent
            + msg_size // actual message byte data
    }
    pub fn from_bytes(bytes: &'a [u8]) -> Self {
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

    pub fn init_at(uninitialized_bytes: &mut [u8], data: &[u8], initialization_timestamp: Instant) {
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

    pub fn to_bytes(&self) -> &[u8] {
        // # Safety
        // len and data should be contiguous memory
        unsafe {
            &*slice_from_raw_parts(
                self.len as *const usize as *const u8,
                Self::size(self.data.len()),
            )
        }
    }

    pub fn get_data(&self) -> &[u8] {
        &self.data[0..*self.len]
    }
}
