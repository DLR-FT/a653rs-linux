use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::SyscallType;

pub trait Syscall<'params> {
    const TY: SyscallType;
    type Params: Serialize + Deserialize<'params>;
    type Returns: Serialize + DeserializeOwned;
}

pub struct SendQueuingMessage;
impl<'msg> Syscall<'msg> for SendQueuingMessage {
    const TY: SyscallType = SyscallType::SendQueuingMessage;
    type Params = &'msg [u8];
    type Returns = ();
}

pub struct ReceiveQueuingMessage;
impl<'de> Syscall<'de> for ReceiveQueuingMessage {
    const TY: SyscallType = SyscallType::ReceiveQueuingMessage;
    type Params = ();
    type Returns = Vec<u8>;
}
