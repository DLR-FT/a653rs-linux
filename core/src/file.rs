use std::ffi::{c_void, CString};
use std::fs::File;
use std::marker::PhantomData;
use std::mem::{size_of, MaybeUninit};
use std::os::unix::io::FromRawFd;
use std::os::unix::prelude::AsRawFd;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use nix::fcntl::{fcntl, FcntlArg, SealFlag};
use nix::libc::{pread, pwrite};
use nix::sys::memfd::{memfd_create, MemFdCreateFlag};
use nix::unistd::{dup, ftruncate};
use procfs::process::{FDTarget, Process};

#[derive(Debug)]
pub struct TempFile<T> {
    file: File,
    _pd: PhantomData<T>,
}

impl<T> TempFile<T> {
    pub fn new(name: &str) -> Result<TempFile<T>> {
        let file = unsafe {
            File::from_raw_fd(memfd_create(
                &CString::new(name)?,
                MemFdCreateFlag::MFD_ALLOW_SEALING,
            )?)
        };
        ftruncate(file.as_raw_fd(), size_of::<T>().try_into()?)?;

        Ok(Self {
            file,
            _pd: Default::default(),
        })
    }

    pub fn from_fd(fd: i32) -> Result<TempFile<T>> {
        let file = unsafe { File::from_raw_fd(dup(fd)?) };
        Ok(TempFile {
            file,
            _pd: Default::default(),
        })
    }

    pub fn write(&self, mut value: T) -> Result<()> {
        let bytes = unsafe {
            pwrite(
                self.file.as_raw_fd(),
                &mut value as *mut T as *mut c_void,
                size_of::<T>(),
                0,
            )
        };

        if bytes != (size_of::<T>() as isize) {
            return Err(anyhow!("Error Writing: {bytes}"));
        }
        Ok(())
    }

    pub fn lock_all(&self) -> Result<()> {
        fcntl(
            self.file.as_raw_fd(),
            FcntlArg::F_ADD_SEALS(
                SealFlag::F_SEAL_GROW
                    | SealFlag::F_SEAL_SHRINK
                    | SealFlag::F_SEAL_WRITE
                    | SealFlag::F_SEAL_SEAL,
            ),
        )?;
        Ok(())
    }

    pub fn lock_trunc(&self) -> Result<()> {
        fcntl(
            self.file.as_raw_fd(),
            FcntlArg::F_ADD_SEALS(
                SealFlag::F_SEAL_GROW | SealFlag::F_SEAL_SHRINK | SealFlag::F_SEAL_SEAL,
            ),
        )?;
        Ok(())
    }

    pub fn read(&self) -> Result<T> {
        let res: MaybeUninit<T> = MaybeUninit::uninit();
        let res_ptr = res.as_ptr() as *mut c_void;
        unsafe {
            let bytes = pread(self.file.as_raw_fd(), res_ptr, size_of::<T>(), 0);

            if bytes != (size_of::<T>() as isize) {
                return Err(anyhow!("Error Reading: {bytes}"));
            }
            Ok(res.assume_init())
        }
    }

    pub fn get_fd(&self) -> i32 {
        self.file.as_raw_fd()
    }
}

pub fn get_fd(name: &str) -> Result<i32> {
    Process::myself()?
        .fd()?
        .flatten()
        .find_map(|f| {
            if let FDTarget::Path(p) = &f.target {
                if p.to_str().unwrap().contains(name) {
                    Some(f.fd)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow!("No File Descriptor with Name: {name}"))
}
