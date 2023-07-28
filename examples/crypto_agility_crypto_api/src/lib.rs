pub mod client;
pub mod server;

#[repr(u8)]
#[derive(num_derive::FromPrimitive, Clone, Copy, Debug)]
pub enum OpCode {
    Encrypt = 1,
    Decrypt = 2,
    RequestPeerEncryptionKey = 3,
    RequestPeerSignatureKey = 4,
}

#[repr(u8)]
#[derive(num_derive::FromPrimitive, Clone, Copy, Debug)]
pub enum ResultCode {
    Error = 0,
    Ok = 1,
}
