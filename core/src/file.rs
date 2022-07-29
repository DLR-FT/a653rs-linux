use anyhow::{anyhow, Result};
use nix::{
    fcntl::{fcntl, FcntlArg, SealFlag},
    libc::{pread, pwrite},
    sys::memfd::{memfd_create, MemFdCreateFlag},
    unistd::ftruncate,
};
use std::{
    ffi::{c_void, CString},
    marker::PhantomData,
    mem::{size_of, MaybeUninit},
};

pub struct TempFile<T> {
    fd: i32,
    _pd: PhantomData<T>,
}

impl<T> TempFile<T> {
    pub fn new(name: &str) -> Result<TempFile<T>> {
        let fd = memfd_create(&CString::new(name)?, MemFdCreateFlag::MFD_ALLOW_SEALING)?;
        ftruncate(fd, size_of::<T>().try_into()?)?;

        Ok(Self {
            fd,
            _pd: Default::default(),
        })
    }

    pub fn from_fd(fd: i32) -> TempFile<T> {
        TempFile {
            fd,
            _pd: Default::default(),
        }
    }

    pub fn write(&self, mut value: T) -> Result<()> {
        let bytes = unsafe {
            pwrite(
                self.fd,
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
            self.fd,
            FcntlArg::F_ADD_SEALS(
                SealFlag::F_SEAL_GROW
                    | SealFlag::F_SEAL_SHRINK
                    | SealFlag::F_SEAL_WRITE
                    | SealFlag::F_SEAL_SEAL,
            ),
        )?;
        Ok(())
    }

    pub fn read(&self) -> Result<T> {
        let res: MaybeUninit<T> = MaybeUninit::uninit();
        let res_ptr = res.as_ptr() as *mut c_void;
        unsafe {
            let bytes = pread(self.fd, res_ptr, size_of::<T>(), 0);

            if bytes != (size_of::<T>() as isize) {
                return Err(anyhow!("Error Reading: {bytes}"));
            }
            Ok(res.assume_init())
        }
    }

    pub fn get_fd(&self) -> i32 {
        self.fd
    }
}
