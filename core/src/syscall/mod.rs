use nix::sys::ptrace::traceme;
use nix::sys::signal::{raise, Signal};

pub mod ptrace;

// Make child process also traced
// (Or dont because we control all systemcalls)
// PTRACE_O_TRACEFORK / PTRACE_O_TRACEVFORK / PTRACE_O_TRACECLONE

#[non_exhaustive]
pub enum ApexSyscall {
    /// P1-5 3.2.2.1 - GET_PARTITION_STATUS
    GetPartitionStatus = 6530,
    /// P1-5 3.2.2.2 - SET_PARTITION_MODE
    SetPartitionMode = 6531,
    /////////////////////
    //* 10 Free Spaces */
    /////////////////////
    /// P1-5 3.3.2.1 - GET_PROCESS_ID
    GetProcessId = 6542,
    /// P1-5 3.3.2.2 - GET_PROCESS_STATUS
    GetProcessStatus = 6543,
    /// P1-5 3.3.2.3 - CREATE_PROCESS
    CreateProcess = 6544,
    /// P1-5 3.3.2.4 - SET_PRIORITY
    SetPriority = 6545,
    /// P1-5 3.3.2.5 - SUSPEND_SELF
    SuspendSelf = 6546,
    /// P1-5 3.3.2.6 - SUSPEND
    Suspend = 6547,
    /// P1-5 3.3.2.7 - RESUME
    Resume = 6548,
    /// P1-5 3.3.2.8 - STOP_SELF
    StopSelf = 6549,
    /// P1-5 3.3.2.9 - STOP
    Stop = 6550,
    /// P1-5 3.3.2.10 - START
    Start = 6551,
    /// P1-5 3.3.2.11 - DELAY_START
    DelayStart = 6552,
    /// P1-5 3.3.2.12 - LOCK_PREEMPTION
    LockPreemption = 6553,
    /// P1-5 3.3.2.13 - UNLOCK_PREEMPTION
    UnlockPreemption = 6554,
    /// P1-5 3.3.2.14 - GET_MY_ID
    GetMyId = 6555,
    /// P1-5 3.3.2.15 - INITIALIZE_PROCESS_CORE_AFFINITY
    InitializeProcessCoreAffinity = 6556,
    /// P1-5 3.3.2.16 - GET_MY_PROCESSOR_CORE_ID
    GetMyProcessorCoreId = 6557,
    /// P1-5 3.3.2.17 - GET_MY_INDEX
    GetMyIndex = 6558,
    /////////////////////
    //* 10 Free Spaces */
    /////////////////////
    /// P1-5 3.4.2.1 - TIMED_WAIT
    TimedWait = 6569,
    /// P1-5 3.4.2.2 - PERIODIC_WAIT
    PeriodicWait = 6570,
    /// P1-5 3.4.2.3 - GET_TIME
    GetTime = 6571,
    /// P1-5 3.4.2.4 - REPLENISH
    Replenish = 6572,
    /////////////////////
    //* 10 Free Spaces */
    /////////////////////
    /// P1-5 3.6.2.1.1 - CREATE_SAMPLING_PORT
    CreateSamplingPort = 6583,
    /// P1-5 3.6.2.1.2 - WRITE_SAMPLING_MESSAGE
    WriteSamplingMessage = 6584,
    /// P1-5 3.6.2.1.3 - READ_SAMPLING_MESSAGE
    ReadSamplingMessage = 6585,
    /// P1-5 3.6.2.1.4 - GET_SAMPLING_MESSAGE_PORT_ID
    GetSamplingMessagePortId = 6586,
    /// P1-5 3.6.2.1.5 - GET_SAMPLING_PORT_STATUS
    GetSamplingPortStatus = 6587,
    /////////////////////
    //* 10 Free Spaces */
    /////////////////////
    /// P1-5 3.6.2.1.1 - CREATE_QUEUING_PORT
    CreateQueuingPort = 6598,
    /// P1-5 3.6.2.1.2 - SEND_QUEUING_MESSAGE
    SendQueuingMessage = 6599,
    /// P1-5 3.6.2.1.3 - RECEIVE_QUEUING_MESSAGE
    ReceiveQueuingMessage = 6600,
    /// P1-5 3.6.2.1.4 - GET_QUEUING_MESSAGE_PORT_ID
    GetQueuingMessagePortId = 6601,
    /// P1-5 3.6.2.1.5 - GET_QUEUING_PORT_STATUS
    GetQueuingPortStatus = 6602,
    /// P1-5 3.6.2.1.6 - CLEAR_QUEUING_PORT
    ClearQueuingPort = 6603,
    /////////////////////
    //* 10 Free Spaces */
    /////////////////////
    /// P1-5 3.7.2.1.1 - CREATE_BUFFER
    CreateBuffer = 6614,
    /// P1-5 3.7.2.1.2 - SEND_BUFFER
    SendBuffer = 6615,
    /// P1-5 3.7.2.1.3 - RECEIVE_BUFFER
    ReceiveBuffer = 6616,
    /// P1-5 3.7.2.1.4 - GET_BUFFER_ID
    GetBufferId = 6617,
    /// P1-5 3.7.2.1.5 - GET_BUFFER_STATUS
    GetBufferStatus = 6618,
    /////////////////////
    //* 10 Free Spaces */
    /////////////////////
    /// P1-5 3.7.2.2.1 - CREATE_BLACKBOARD
    CreateBlackboard = 6629,
    /// P1-5 3.7.2.2.2 - DISPLAY_BLACKBOARD
    DisplayBlackboard = 6630,
    /// P1-5 3.7.2.2.3 - READ_BLACKBOARD
    ReadBlackboard = 6631,
    /// P1-5 3.7.2.2.4 - CLEAR_BLACKBOARD
    ClearBlackboard = 6632,
    /// P1-5 3.7.2.2.5 - GET_BLACKBOARD_ID
    GetBlackboardId = 6633,
    /// P1-5 3.7.2.2.6 - GET_BLACKBOARD_STATUS
    GetBlackboardStatus = 6634,
    /////////////////////
    //* 10 Free Spaces */
    /////////////////////
    /// P1-5 3.7.2.3.1 - CREATE_SEMAPHORE
    CreateSemaphore = 6645,
    /// P1-5 3.7.2.3.2 - WAIT_SEMAPHORE
    WaitSemaphore = 6646,
    /// P1-5 3.7.2.3.3 - SIGNAL_SEMAPHORE
    SignalSemaphore = 6647,
    /// P1-5 3.7.2.3.4 - GET_SEMAPHORE_ID
    GetSemaphoreId = 6648,
    /// P1-5 3.7.2.3.5 - GET_SEMAPHORE_STATUS
    GetSemaphoreStatus = 6649,
    /////////////////////
    //* 10 Free Spaces */
    /////////////////////
    /// P1-5 3.7.2.4.1 - CREATE_EVENT
    CreateEvent = 6660,
    /// P1-5 3.7.2.4.2 - SET_EVENT
    SetEvent = 6661,
    /// P1-5 3.7.2.4.3 - RESET_EVENT
    ResetEvent = 6662,
    /// P1-5 3.7.2.4.4 - WAIT_EVENT
    WaitEvent = 6663,
    /// P1-5 3.7.2.4.5 - GET_EVENT_ID
    GetEventId = 6664,
    /// P1-5 3.7.2.4.6 - GET_EVENT_STATUS
    GetEventStatus = 6665,
    /////////////////////
    //* 10 Free Spaces */
    /////////////////////
    /// P1-5 3.7.2.1.1 - CREATE_MUTEX
    CreateMutex = 6676,
    /// P1-5 3.7.2.1.2 - ACQUIRE_MUTEX
    AcquireMutex = 6677,
    /// P1-5 3.7.2.1.3 - RELEASE_MUTEX
    ReleaseMutex = 6678,
    /// P1-5 3.7.2.1.4 - RESET_MUTEX
    ResetMutex = 6679,
    /// P1-5 3.7.2.1.5 - GET_MUTEX_ID
    GetMutexId = 6680,
    /// P1-5 3.7.2.1.6 - GET_MUTEX_STATUS
    GetMutexStatus = 6681,
    /// P1-5 3.7.2.1.7 - GET_PROCIESS_MUTEX_STATE
    GetProciessMutexState = 6682,
    /////////////////////
    //* 10 Free Spaces */
    /////////////////////
    /// P1-5 3.8.2.1 - REPORT_APPLICATION_MESSAGE
    ReportApplicationMessage = 6693,
    /// P1-5 3.8.2.2 - CREATE_ERROR_HANDLER
    CreateErrorHandler = 6694,
    /// P1-5 3.8.2.3 - GET_ERROR_STATUS
    GetErrorStatus = 6695,
    /// P1-5 3.8.2.4 - RAISE_APPLICATION_ERROR
    RaiseApplicationError = 6696,
    /// P1-5 3.8.2.5 - CONFIGURE_ERROR_HANDLER
    ConfigureErrorHandler = 6697,
}

impl ApexSyscall {
    pub fn install() -> anyhow::Result<()> {
        traceme()?;
        raise(Signal::SIGSTOP)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::c_void;

    use nix::libc;
    use nix::sys::ptrace::{self, attach, cont, getregs, syscall};
    use nix::sys::signal::Signal;
    use nix::sys::wait::waitpid;
    use nix::unistd::{fork, write, ForkResult};

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn custom_syscall() {}

    // #[test]
    // fn custom_systemcall() {
    //     match unsafe { fork() } {
    //         Ok(ForkResult::Parent { child, .. }) => {
    //             // attach(child).unwrap();
    //             let res: Vec<_> = (0..100)
    //                 .map(|_| {
    //                     let res1 = waitpid(child, None);
    //                     // cont(child, None).ok();
    //                     let res2 = getregs(child).map(|r| (r.orig_rax, r.rdi, r.rsi, r.rdx, r.r10));

    //                     if let Ok(regs) = getregs(child) {
    //                         if regs.orig_rax == 500 {
    //                             // let data = 5;
    //                             unsafe {
    //                                 ptrace::write(
    //                                     child,
    //                                     regs.rdi as *mut c_void,
    //                                     5u64 as *mut u64 as *mut c_void,
    //                                 )
    //                             };
    //                         }
    //                     }

    //                     // syscall(child, None).ok();
    //                     // let res3 = waitpid(child, None);

    //                     // sysemu(child, None).ok();
    //                     syscall(child, None).ok();
    //                     (res1, res2)
    //                 })
    //                 .collect();
    //             panic!("{res:?}");
    //         }
    //         Ok(ForkResult::Child) => {
    //             ApexSyscall::install().unwrap();
    //             unsafe {
    //                 let mut test = [0u64, 0, 0, 0];
    //                 let res = nix::libc::syscall(500, &mut test);
    //                 nix::libc::syscall(501, test[0], test[1], test[2], test[3]);
    //             };
    //             unsafe { fork() }.unwrap();
    //             // unsafe { libc::_exit(0) };

    //             raise(Signal::SIGKILL).unwrap();
    //         }
    //         Err(_) => println!("Fork failed"),
    //     };
    //     assert!(true);
    // }
}
