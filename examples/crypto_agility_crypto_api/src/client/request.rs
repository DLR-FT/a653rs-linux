use core::ops::{Deref, DerefMut};

use crate::{OpCode, SizedSliceField};

use super::{Error, Key};

pub struct RequestBuilder<'a> {
    buffer: &'a mut [u8],
    len: usize,
}

impl<'a> Deref for RequestBuilder<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.buffer
    }
}

impl<'a> DerefMut for RequestBuilder<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.buffer
    }
}

impl<'a> RequestBuilder<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        Self { buffer, len: 0 }
    }

    fn available_space(&self) -> usize {
        self.buffer.len() - self.len
    }

    fn add_op_code(&mut self, op: OpCode) -> Result<(), Error> {
        if self.buffer.is_empty() {
            return Err(Error::OutOfSpace);
        }
        self.buffer[0] = op as u8;
        self.len = 1;
        Ok(())
    }

    fn add_key<const MAX_KEY_SIZE: usize>(
        &mut self,
        public_key: &Key<MAX_KEY_SIZE>,
    ) -> Result<(), Error> {
        if self.available_space() < public_key.len() {
            return Err(Error::OutOfSpace);
        }
        self.buffer[self.len..self.len + public_key.len()].copy_from_slice(public_key.read());
        self.len += public_key.len();
        Ok(())
    }

    fn add_peer_id(&mut self, peer_id: u32) -> Result<(), Error> {
        let peer_bytes = peer_id.to_le_bytes();
        if self.available_space() < peer_bytes.len() {
            return Err(Error::OutOfSpace);
        }
        self.buffer[self.len..self.len + peer_bytes.len()].copy_from_slice(&peer_bytes);
        self.len += peer_bytes.len();
        Ok(())
    }

    fn add_payload(&mut self, payload: &[u8]) -> Result<(), Error> {
        if self.available_space() < payload.len() {
            return Err(Error::OutOfSpace);
        }
        self.buffer[self.len..self.len + payload.len()].copy_from_slice(payload);
        self.len += payload.len();
        Ok(())
    }

    fn add_additional_data(&mut self, additional_data: &[u8]) -> Result<(), Error> {
        let add_size_field_size = core::mem::size_of::<u32>();
        let add_size = additional_data.len();
        let required_size = add_size_field_size + add_size;
        if self.available_space() < required_size {
            return Err(Error::OutOfSpace);
        }
        self.buffer[self.len..self.len + required_size].insert_sized_field(additional_data);
        self.len += required_size;
        Ok(())
    }

    /// build peer public encryption key request
    ///
    /// this function returns the final request message
    pub fn build_peer_public_key_request(&mut self, peer_id: u32) -> Result<&[u8], Error> {
        self.add_op_code(OpCode::RequestPeerPublicKey)?;
        self.add_peer_id(peer_id)?;

        Ok(&self.buffer[..self.len])
    }

    /// build an encryption request
    ///
    /// this function returns the final request message
    pub fn build_encryption_request<const MAX_KEY_SIZE: usize>(
        &mut self,
        public_encryption_key: &Key<MAX_KEY_SIZE>,
        msg: &[u8],
        additional_data: &[u8],
    ) -> Result<&[u8], Error> {
        self.add_op_code(OpCode::Encrypt)?;
        self.add_additional_data(additional_data)?;
        self.add_key(public_encryption_key)?;
        self.add_payload(msg)?;

        Ok(&self.buffer[..self.len])
    }

    /// build a decryption request
    ///
    /// this function returns the final request message
    pub fn build_decrypt_request<const MAX_KEY_SIZE: usize>(
        &mut self,
        public_sign_key: &Key<MAX_KEY_SIZE>,
        encrypted_message: &[u8],
        additional_data: &[u8],
    ) -> Result<&[u8], Error> {
        self.add_op_code(OpCode::Decrypt)?;
        self.add_additional_data(additional_data)?;
        self.add_key(public_sign_key)?;
        self.add_payload(encrypted_message)?;

        Ok(&self.buffer[..self.len])
    }
}
