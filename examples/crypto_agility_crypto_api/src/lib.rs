use anyhow::bail;

pub mod client;
pub mod server;

#[repr(u8)]
#[derive(num_derive::FromPrimitive, Clone, Copy, Debug)]
pub enum OpCode {
    Encrypt = 1,
    Decrypt = 2,
    RequestPeerPublicKey = 3,
}

#[repr(u8)]
#[derive(num_derive::FromPrimitive, Clone, Copy, Debug)]
pub enum ResultCode {
    Error = 0,
    Ok = 1,
}

pub trait SizedSliceField {
    fn extract_sized_field(&self) -> Result<(&[u8], &[u8]), anyhow::Error>;
    fn insert_sized_field(&mut self, field: &[u8]);
}

impl SizedSliceField for [u8] {
    fn extract_sized_field(&self) -> Result<(&[u8], &[u8]), anyhow::Error> {
        let field_size_field_size = core::mem::size_of::<u32>();
        if self.len() < field_size_field_size {
            bail!("extract field error: slice is shorter than u32");
        }
        let (field_size_buffer, rest) = self.split_at(field_size_field_size);
        let field_size = u32::from_le_bytes(field_size_buffer.try_into()?) as usize;
        if rest.len() < field_size {
            bail!(
                "extract field error: slice({}) is shorter than size field says: {field_size}",
                rest.len()
            )
        }
        Ok(rest.split_at(field_size))
    }

    fn insert_sized_field(&mut self, field: &[u8]) {
        let field_size = field.len();
        let field_size_field_size = core::mem::size_of::<u32>();
        let (size_buffer, field_buffer) = self.split_at_mut(field_size_field_size);
        size_buffer.copy_from_slice((field_size as u32).to_le_bytes().as_slice());
        field_buffer[..field_size].copy_from_slice(field);
    }
}
