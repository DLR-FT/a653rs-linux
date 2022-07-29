use std::{ffi::CString, mem::size_of, ptr::null_mut};

use anyhow::Result;
use nix::{
    fcntl::{fcntl, FcntlArg, SealFlag},
    sys::{
        memfd::{memfd_create, MemFdCreateFlag},
        mman::{mmap, MapFlags, ProtFlags},
    },
    unistd::ftruncate,
};

pub struct Shmem<T> {
    fd: i32,
    ptr: *mut T,
}

impl<T> Shmem<T> {
    /// .
    ///
    /// # Safety
    ///
    /// .
    pub unsafe fn new(name: &str, init: T) -> Result<Self> {
        let fd = memfd_create(&CString::new(name)?, MemFdCreateFlag::MFD_ALLOW_SEALING)?;
        ftruncate(fd, (size_of::<T>()).try_into()?)?;
        fcntl(
            fd,
            FcntlArg::F_ADD_SEALS(
                SealFlag::F_SEAL_GROW | SealFlag::F_SEAL_SHRINK | SealFlag::F_SEAL_SEAL,
            ),
        )
        .unwrap();

        let mut shmem = Self::from_fd(fd)?;
        shmem.set(init);
        Ok(shmem)
    }

    /// .
    ///
    /// # Safety
    ///
    /// .
    pub unsafe fn from_fd(fd: i32) -> Result<Self> {
        Ok(Self {
            fd,
            ptr: mmap(
                null_mut(),
                size_of::<T>(),
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                MapFlags::MAP_SHARED,
                fd,
                0,
            )? as *mut T,
        })
    }

    /// .
    ///
    /// # Safety
    ///
    /// .
    pub unsafe fn get(&self) -> &T {
        &*self.ptr
    }

    /// .
    ///
    /// # Safety
    ///
    /// .
    pub unsafe fn set(&mut self, value: T) {
        *self.ptr = value
    }

    pub fn ptr(&self) -> *mut T {
        self.ptr
    }

    pub fn fd(&self) -> i32 {
        self.fd
    }
}
