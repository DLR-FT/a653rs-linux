use std::io::{Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::mem::size_of;
use std::os::unix::prelude::{AsRawFd, IntoRawFd, RawFd};

use anyhow::{anyhow, Ok, Result};
use memfd::{FileSeal, Memfd, MemfdOptions};
use memmap2::{Mmap, MmapMut};
use nix::unistd::{close, dup};
use procfs::process::{FDTarget, Process};

use crate::shmem::{TypedMmap, TypedMmapMut};

#[derive(Debug, Clone, Copy)]
pub struct TempFile<T: Send + Copy + Sized> {
    fd: RawFd,
    _p: PhantomData<T>,
}

impl<T: Send + Copy + Sized> TempFile<T> {
    pub fn new(name: &str) -> Result<Self> {
        let mem = MemfdOptions::default()
            .close_on_exec(false)
            .allow_sealing(true)
            .create(name)?;
        mem.as_file().set_len(size_of::<T>().try_into()?)?;
        mem.add_seals(&[FileSeal::SealShrink, FileSeal::SealGrow])?;

        Ok(Self {
            fd: mem.into_raw_fd(),
            _p: PhantomData,
        })
    }

    pub fn from_fd(fd: RawFd) -> Result<Self> {
        //let fd = dup(fd)?;
        let tf = Self {
            fd,
            _p: PhantomData,
        };
        tf.get_memfd()?;
        Ok(tf)
    }

    fn get_memfd(&self) -> Result<Memfd> {
        let fd = dup(self.fd)?;
        Memfd::try_from_fd(fd).map_err(|e| {
            close(fd).ok();
            anyhow!("Could not get Memfd from {e:#?}")
        })
    }

    //fn verified_memfd(&self) -> Result<Memfd>{
    //    let expected_len = size_of::<T>().try_into()?;
    //    let mem = self.get_memfd()?;
    //    let is_len = mem.as_file().metadata()?.len();
    //    if is_len != expected_len{
    //        return Err(anyhow!("Mismatch size. Expected: {is_len}, Is: {expected_len}"));
    //    }
    //    Ok(mem)
    //}

    pub fn seal(&mut self) -> Result<()> {
        self.get_memfd()?.add_seal(FileSeal::SealSeal)?;
        Ok(())
    }

    pub fn seal_read_only(&self) -> Result<()> {
        self.get_memfd()?
            .add_seals(&[FileSeal::SealWrite, FileSeal::SealSeal])?;
        Ok(())
    }

    pub fn fd(&self) -> RawFd {
        self.fd
    }

    // This is kinda unsafe.
    // Map file instead with mmap
    pub fn write(&self, value: &T) -> Result<()> {
        let bytes =
            unsafe { std::slice::from_raw_parts(value as *const T as *const u8, size_of::<T>()) };
        let mut file = self.get_memfd()?.into_file();
        file.seek(SeekFrom::Start(0))?;
        file.write_all(bytes).map_err(|e| anyhow!("{e:#?}"))?;
        Ok(())
    }

    pub fn read(&self) -> Result<T> {
        let mut buf = Vec::with_capacity(size_of::<T>());
        let mut file = self.get_memfd()?.into_file();
        file.seek(SeekFrom::Start(0))?;
        file.read_to_end(buf.as_mut())?;
        Ok(unsafe { buf.as_slice().align_to::<T>().1[0] })
        //Ok(*bytemuck::try_from_bytes(buf.as_slice()).map_err(|e| anyhow!("{e:#?}"))?)
    }

    // TODO Into Mmap
}

impl<T: Send + Copy + Sized> AsRawFd for TempFile<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

pub fn get_memfd(name: &str) -> Result<i32> {
    let name = format!("memfd:{name}");
    Process::myself()?
        .fd()?
        .flatten()
        .find_map(|f| {
            if let FDTarget::Path(p) = &f.target {
                if p.to_str().unwrap().contains(&name) {
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

impl<T: Send + Copy + Sized> TryFrom<&TempFile<T>> for TypedMmapMut<T> {
    type Error = anyhow::Error;

    fn try_from(value: &TempFile<T>) -> Result<Self, Self::Error> {
        let fd = dup(value.fd)?;
        unsafe {
            MmapMut::map_mut(fd)
                .map_err(|e| {
                    close(fd).ok();
                    anyhow!("Could not get Mmap from {e:#?}")
                })?
                .try_into()
        }
    }
}

impl<T: Send + Copy + Sized> TryFrom<&TempFile<T>> for TypedMmap<T> {
    type Error = anyhow::Error;

    fn try_from(value: &TempFile<T>) -> Result<Self, Self::Error> {
        let fd = dup(value.fd)?;
        unsafe {
            Mmap::map(fd)
                .map_err(|e| {
                    close(fd).ok();
                    anyhow!("Could not get Mmap from {e:#?}")
                })?
                .try_into()
        }
    }
}
