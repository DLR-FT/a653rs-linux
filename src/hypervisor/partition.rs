use std::ffi::CString;
use std::fmt::format;
use std::mem::{forget, size_of};
use std::ops::Add;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::prelude::{RawFd, OsStrExt};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use super::cgroup::CGroup;
use anyhow::Result;
use clone3::Clone3;
use ipc_channel::ipc::{self, IpcSender, IpcReceiver};
use libc::{SYS_vserver, MFD_CLOEXEC, MFD_ALLOW_SEALING};
use nix::mount::{mount, MsFlags};
use nix::mqueue::{mq_open, MQ_OFlag, MqAttr};
use nix::sched::{clone, unshare, CloneFlags};
use nix::sys::memfd::{memfd_create, MemFdCreateFlag};
use nix::sys::socket::SockFlag;
use nix::sys::stat::Mode;
use nix::unistd::{chroot, close};
use nix::unistd::{mkdir, Pid};
use raw_sync::locks::*;
use shared_memory::{ShmemConf, Shmem};
use smol::future::FutureExt;
use smol::Timer;
use std::io::{Read, Write};
use tempfile::{tempdir, TempDir};

#[derive(Debug)]
pub(crate) struct Partition {
    name: String,
    cg: CGroup,
    wd: TempDir,
    entry: fn(),
    // TODO Error on closed sender for receiver
    sys_call: (IpcSender<usize>, IpcReceiver<usize>)
}

impl Partition {
    pub fn from_cgroup<P: AsRef<Path>>(cgroup_root: P, name: &str, entry: fn()) -> Result<Self> {
        // Todo implement drop for cgroup? (in error case)
        let cg = CGroup::new(cgroup_root, name)?;
        let wd = tempdir()?;
        println!("Path: {:?}", wd.path());
        std::fs::create_dir(wd.path().join("proc"))?;
        let sys_call = ipc_channel::ipc::channel()?;

        Ok(Self {
            cg,
            entry,
            name: name.to_string(),
            wd,
            sys_call,
        })
    }

    pub fn freeze(&mut self) -> Result<()> {
        self.cg.freeze()
    }

    pub fn unfreeze(&mut self) -> Result<()> {
        self.cg.unfreeze()
    }

    pub fn delete(self) -> Result<()> {
        self.cg.delete()
    }

    pub fn initialize(&mut self) {
        //TODO kill everything in cgroup
        
        self.freeze().unwrap();

        let pid = match unsafe {
            Clone3::default()
                .flag_newuser()
                .flag_newpid()
                .flag_newns()
                .flag_newipc()
                .flag_newnet()
                .flag_into_cgroup(&self.cg.get_fd().unwrap())
                .call()
        }
        .unwrap()
        {
            0 => {
                
                //forget(server);
                //println!("{:#?}", procfs::process::Process::myself().unwrap().fd_count().unwrap());
                //for fd in procfs::process::Process::myself().unwrap().fd().unwrap(){
                //    println!("{:#?}", fd);
                //}
                //for fd in procfs::process::Process::myself().unwrap().fd().unwrap().skip(3){
                //    close(fd.unwrap().fd).unwrap();
                //}

                mount::<str, _, str, str>(
                    Some("/proc"),
                    &self.wd.path().join("proc"),
                    Some("proc"),
                    MsFlags::empty(),
                    None,
                )
                .unwrap();

                chroot(self.wd.path()).unwrap();

                (self.entry)();

                unsafe { libc::_exit(0) };
            }
            child => child,
        };
        println!("Child Pid: {pid}");
        //forget(client);
    }
}

