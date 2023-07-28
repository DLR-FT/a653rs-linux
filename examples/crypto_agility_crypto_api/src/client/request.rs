use crate::OpCode;

use super::{Error, Key};

pub struct RequestBuilder<'a> {
    buffer: &'a mut [u8],
    len: usize,
}

impl<'a> RequestBuilder<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        Self { buffer, len: 0 }
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
        if self.buffer.len() - self.len < public_key.len() {
            return Err(Error::OutOfSpace);
        }
        self.buffer[self.len..self.len + public_key.len()].copy_from_slice(public_key.read());
        self.len += public_key.len();
        Ok(())
    }

    fn add_peer_id(&mut self, peer_id: u32) -> Result<(), Error> {
        let peer_bytes = peer_id.to_le_bytes();
        if self.buffer.len() - self.len < peer_bytes.len() {
            return Err(Error::OutOfSpace);
        }
        self.buffer[self.len..self.len + peer_bytes.len()].copy_from_slice(&peer_bytes);
        self.len += peer_bytes.len();
        Ok(())
    }

    fn add_payload(&mut self, payload: &[u8]) -> Result<(), Error> {
        if self.buffer.len() - self.len < payload.len() {
            return Err(Error::OutOfSpace);
        }
        self.buffer[self.len..self.len + payload.len()].copy_from_slice(payload);
        self.len += payload.len();
        Ok(())
    }

    /// build peer public encryption key request
    ///
    /// this function returns the final request message
    pub fn build_peer_public_encryption_key_request(
        &'a mut self,
        peer_id: u32,
    ) -> Result<&'a [u8], Error> {
        self.add_op_code(OpCode::RequestPeerEncryptionKey)?;
        self.add_peer_id(peer_id)?;

        Ok(&self.buffer[..self.len])
    }

    /// build peer public singature key request
    ///
    /// this function returns the final request message
    pub fn build_peer_public_signature_key_request(
        &'a mut self,
        peer_id: u32,
    ) -> Result<&'a [u8], Error> {
        self.add_op_code(OpCode::RequestPeerSignatureKey)?;
        self.add_peer_id(peer_id)?;

        Ok(&self.buffer[..self.len])
    }

    /// build an encryption request
    ///
    /// this function returns the final request message
    pub fn build_encryption_request<const MAX_KEY_SIZE: usize>(
        &'a mut self,
        public_encryption_key: &Key<MAX_KEY_SIZE>,
        msg: &[u8],
    ) -> Result<&'a [u8], Error> {
        self.add_op_code(OpCode::Encrypt)?;
        self.add_key(public_encryption_key)?;
        self.add_payload(msg)?;

        Ok(&self.buffer[..self.len])
    }

    /// build a decryption request
    ///
    /// this function returns the final request message
    pub fn build_decrypt_request<const MAX_KEY_SIZE: usize>(
        &'a mut self,
        public_sign_key: &Key<MAX_KEY_SIZE>,
        encrypted_message: &[u8],
    ) -> Result<&'a [u8], Error> {
        self.add_op_code(OpCode::Decrypt)?;
        self.add_key(public_sign_key)?;
        self.add_payload(encrypted_message)?;

        Ok(&self.buffer[..self.len])
    }
}
