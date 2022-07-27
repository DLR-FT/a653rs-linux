use std::ffi::CString;
use std::fmt::format;
use std::fs::{read_to_string, File};
use std::mem::{MaybeUninit, size_of};
use std::os::unix::prelude::{RawFd, AsRawFd, CommandExt};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::ptr::null_mut;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::thread::sleep;
use std::time::Duration;

use super::cgroup::CGroup;
use anyhow::Result;
use clone3::Clone3;
use itertools::Itertools;
use libc::{c_void, munlock};
use nix::fcntl::{fcntl, FcntlArg, SealFlag};
//use ipc_channel::ipc::{IpcReceiver, IpcSender};
use nix::mount::{mount, MsFlags};
use nix::sys::memfd::{memfd_create, MemFdCreateFlag};
use nix::sys::mman::{mmap, ProtFlags, MapFlags, mlock};
use nix::unistd::{chroot, setuid, Uid, setgid, Gid, ftruncate, close, fork, chdir};
use procfs::process::{Process, FDInfo, FDTarget};
use shared_memory::{ShmemConf, Shmem};
use tempfile::{tempdir, TempDir};



//#[derive(Debug)]
pub(crate) struct Partition {
    name: String,
    cg: CGroup,
    wd: TempDir,
    shmem: i32,
    bin: PathBuf,
    // TODO Error on closed sender for receiver
    //sys_call: (IpcSender<usize>, IpcReceiver<usize>),
}

impl Partition {
    pub fn from_cgroup<P1: AsRef<Path>, P2: AsRef<Path>>(cgroup_root: P1, name: &str, bin: P2) -> Result<Self> {
        // Todo implement drop for cgroup? (in error case)
        let cg = CGroup::new(cgroup_root, name)?;
        let wd = tempdir()?;
        println!("Path: {:?}", wd.path());
        let shmem = memfd_create(&CString::new(format!("{name}-shmem"))?, MemFdCreateFlag::MFD_ALLOW_SEALING)?;
        ftruncate(shmem, size_of::<u8>() as i64)?;
        fcntl(shmem, FcntlArg::F_ADD_SEALS(SealFlag::F_SEAL_GROW | SealFlag::F_SEAL_SHRINK | SealFlag::F_SEAL_SEAL)).unwrap();

        for fd in Process::myself().unwrap().fd().unwrap(){
            println!("{fd:#?}");
        }
        //
        unsafe {  
            let ip = mmap(null_mut(), 1, ProtFlags::PROT_READ | ProtFlags::PROT_WRITE, MapFlags::MAP_SHARED, shmem, 0).unwrap() as *mut u8;
            let jp = mmap(null_mut(), 1, ProtFlags::PROT_READ | ProtFlags::PROT_WRITE, MapFlags::MAP_SHARED, shmem, 0).unwrap() as *mut u8;

            println!("{ip:?}, {jp:?}");

            let mut i = Pin::new(ip.as_mut().unwrap());
            let mut j = Pin::new(jp.as_mut().unwrap());

            i.set(5);
            println!("Value: {}", *j);
        }

        //let sys_call = ipc_channel::ipc::channel()?;

        Ok(Self {
            cg,
            shmem,
            bin: PathBuf::from(bin.as_ref()),
            name: name.to_string(),
            wd,
            //sys_call,
        })
    }

    fn release_fds(keep: &[i32]) -> Result<()> {
        let proc = Process::myself()?;
        for fd in proc.fd()?.skip(3).flatten().filter(|fd| !keep.contains(&fd.fd)){
            close(fd.fd)?
        }

        Ok(())
    }

    fn print_fds() {
        let fds = Process::myself().unwrap().fd().unwrap();
        for f in fds{
            println!("{f:#?}")
        }
    }

    fn print_mountinfo() {
        let mi = Process::myself().unwrap().mountinfo().unwrap();
        for i in mi{
            println!("{i:#?}")
        }
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
                .flag_newcgroup()
                .flag_newuser()
                .flag_newpid()
                .flag_newns()
                .flag_newipc()
                .flag_newnet()
                .flag_into_cgroup(&self.cg.get_fd().unwrap().as_raw_fd())
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
                setuid(Uid::from_raw(0)).unwrap();
                setgid(Gid::from_raw(0)).unwrap();

                Self::release_fds(&[self.shmem]).unwrap();
                Self::print_fds();

                mount::<str, _, _, str>(
                    None,
                    self.wd.path(),
                    Some("tmpfs"),
                    MsFlags::empty(),
                    //Some("mode=0700,uid=0,size=50000k"),
                    None,
                ).unwrap();

                let bin = self.wd.path().join("bin");
                drop(File::create(&bin).unwrap());
                mount::<_, _, str, str>(
                    Some(&self.bin),
                    &bin,
                    None,
                    MsFlags::MS_RDONLY | MsFlags::MS_BIND,
                    None,
                )
                .unwrap();

                let proc = self.wd.path().join("proc");
                std::fs::create_dir(proc.as_path()).unwrap();
                mount::<str, _, str, str>(
                    Some("/proc"),
                    proc.as_path(),
                    Some("proc"),
                    MsFlags::empty(),
                    None,
                )
                .unwrap();

                let cgroup = self.wd.path().join("sys/fs/cgroup");
                std::fs::create_dir_all(&cgroup).unwrap();
                mount::<str, _, str, str>(
                    //Some(self.child_cg.path().to_str().unwrap()),
                    None,
                    //Some("sys/fs/cgroup"),
                    cgroup.as_path(),
                    Some("cgroup2"),
                    MsFlags::empty(),
                    None,
                )
                .unwrap();

                chroot(self.wd.path()).unwrap();

                Self::print_mountinfo();

                //TODO why can we still access ./ from here?
                let paths = std::fs::read_dir("./").unwrap();
                for path in paths {
                    println!("Name: {}", path.unwrap().path().display())
                }

                println!("{:#?}", File::open("/bin").unwrap().metadata().unwrap().permissions());

                //(self.entry)();
                let err = Command::new("/bin")
                    .arg(&self.name)
                    .exec();

                println!("{err:?}");

                unsafe { libc::_exit(0) };
            }
            child => child,
        };
        println!("Child Pid: {pid}");
        std::fs::write(PathBuf::from("/proc").join(pid.to_string()).join("uid_map"), format!("0 {} 1", nix::unistd::getuid().as_raw())).unwrap();
        std::fs::write(PathBuf::from("/proc").join(pid.to_string()).join("setgroups"), b"deny").unwrap();
        std::fs::write(PathBuf::from("/proc").join(pid.to_string()).join("gid_map"), format!("0 {} 1", nix::unistd::getgid().as_raw()).as_bytes()).unwrap();
        //forget(client);
    }
}
 
struct PartitionContext{
    lock: Mutex<usize>,
    
}

impl PartitionContext{
    

}