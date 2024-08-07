use a653rs::bindings::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub trait Syscall<'params> {
    const TY: super::SyscallType;
    type Params: Serialize + Deserialize<'params>;
    type Returns: Serialize + DeserializeOwned;
}

/// `'params` is available as a lifetime for parameter types
macro_rules! define_syscall {
    ($name:ident: |$params:ty| -> $returns:ty) => {
        pub struct $name;
        impl<'params> Syscall<'params> for $name {
            const TY: super::SyscallType = super::SyscallType::$name;
            type Params = $params;
            type Returns = $returns;
        }
    };
}

macro_rules! define_multiple_syscalls {
    ($($name:ident: |$params:ty| -> $returns:ty),* $(,)?) => {
        $(define_syscall!($name: |$params| -> $returns);)*
    }
}

// ApexPartitionP4
define_multiple_syscalls!(
    GetPartitionStatus: |()| -> ApexPartitionStatus,
    SetPartitionMode: |OperatingMode| -> (),
);

// ApexProcessP4
define_multiple_syscalls!(
    // CreateProcess: |&'params ApexProcessAttribute| -> ProcessId,
    Start: |ProcessId| -> (),
);

// ApexSamplingPortP4
define_multiple_syscalls!(
    CreateSamplingPort: |(SamplingPortName, MessageSize, PortDirection, ApexSystemTime)| -> SamplingPortId,
    WriteSamplingMessage: |(SamplingPortId, &'params [ApexByte])| -> (),
    ReadSamplingMessage: |SamplingPortId| -> (Validity, Vec<u8>),
);

// ApexQueuingPortP4
define_multiple_syscalls!(
    CreateQueuingPort: |(QueuingPortName, MessageSize, MessageRange, PortDirection, QueuingDiscipline)| -> QueuingPortId,
    SendQueuingMessage: |(QueuingPortId, &'params [ApexByte], ApexSystemTime)| -> (),
    ReceiveQueuingMessage: |(QueuingPortId, ApexSystemTime)| -> (QueueOverflow, Vec<u8>),
    GetQueuingPortStatus: |QueuingPortId| -> QueuingPortStatus,
    ClearQueuingPort: |QueuingPortId| -> (),
);

// ApexTimeP4
define_multiple_syscalls!(
    PeriodicWait: |()| -> (),
    GetTime: |()| -> ApexSystemTime,
);

// ApexErrorP4
define_multiple_syscalls!(
    ReportApplicationMessage: |&'params [ApexByte]| -> (),
    RaiseApplicationError: |(ErrorCode, &'params [ApexByte])| -> (),
);
