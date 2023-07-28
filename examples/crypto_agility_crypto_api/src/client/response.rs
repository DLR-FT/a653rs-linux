use crate::{OpCode, ResultCode};

use super::{Error, Key};

use num_traits::FromPrimitive;

#[derive(Debug)]
pub enum Response<'a> {
    OwnSecretKey(KeyResponse<'a>),
    EncryptedMessage(&'a [u8]),
    DecryptedMessage { signed: bool, message: &'a [u8] },
    PeerPublicEncryptionKey(KeyResponse<'a>),
    PeerPublicSignatureKey(KeyResponse<'a>),
    Error { opcode: OpCode, err: &'a str },
}

impl<'a> Response<'a> {
    fn extract_result_code(buffer: &[u8]) -> Result<(ResultCode, &[u8]), ()> {
        let (result_code, rest) = buffer.split_first().ok_or(())?;
        ResultCode::from_u8(*result_code)
            .ok_or(())
            .map(|result_code| (result_code, rest))
    }

    fn extract_op_code(buffer: &[u8]) -> Result<(OpCode, &[u8]), ()> {
        let (op_code, rest) = buffer.split_first().ok_or(())?;
        OpCode::from_u8(*op_code)
            .ok_or(())
            .map(|op_code| (op_code, rest))
    }

    fn parse_ok(opcode: OpCode, buffer: &[u8]) -> Result<Response<'_>, ()> {
        match opcode {
            OpCode::Encrypt => Ok(Response::EncryptedMessage(buffer)),
            OpCode::Decrypt => Self::parse_decrypted_message(buffer),
            OpCode::RequestPeerEncryptionKey => {
                Ok(Response::PeerPublicEncryptionKey(buffer.into()))
            }
            OpCode::RequestPeerSignatureKey => Ok(Response::PeerPublicSignatureKey(buffer.into())),
        }
    }

    fn parse_decrypted_message(response: &[u8]) -> Result<Response<'_>, ()> {
        if response.is_empty() {
            return Err(());
        }
        let (signed, message) = response.split_first().ok_or(())?;
        let signed = *signed == true as u8;
        Ok(Response::DecryptedMessage { signed, message })
    }

    fn parse_err(opcode: OpCode, buffer: &'a [u8]) -> Result<Response<'a>, ()> {
        core::str::from_utf8(buffer)
            .map(|err| Response::Error { opcode, err })
            .map_err(|_| ())
    }
}

#[derive(Debug)]
pub struct KeyResponse<'a> {
    key: &'a [u8],
}

impl<'a> KeyResponse<'a> {
    pub fn into_key<const MAX_KEY_SIZE: usize>(self) -> Result<Key<MAX_KEY_SIZE>, Error> {
        self.key.try_into()
    }
}

impl<'a> From<&'a [u8]> for KeyResponse<'a> {
    fn from(key: &'a [u8]) -> Self {
        Self { key }
    }
}

impl<'a> TryFrom<&'a [u8]> for Response<'a> {
    type Error = ();

    fn try_from(buffer: &'a [u8]) -> Result<Self, ()> {
        let (result_code, rest) = Self::extract_result_code(buffer)?;
        let (op_code, rest) = Self::extract_op_code(rest)?;
        match result_code {
            ResultCode::Error => Self::parse_err(op_code, rest),
            ResultCode::Ok => Self::parse_ok(op_code, rest),
        }
    }
}
