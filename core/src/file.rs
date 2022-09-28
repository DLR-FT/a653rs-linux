use std::io::{Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::mem::size_of;
use std::os::unix::prelude::{AsRawFd, IntoRawFd, RawFd};

use anyhow::{anyhow, Result};
use memfd::{FileSeal, Memfd, MemfdOptions};
use memmap2::{Mmap, MmapMut};
use nix::unistd::{close, dup};
use procfs::process::{FDTarget, Process};

use crate::error::{ResultExt, SystemError, TypedError, TypedResult};
use crate::shmem::{TypedMmap, TypedMmapMut};

#[derive(Debug, Clone, Copy)]
pub struct TempFile<T: Send + Clone + Sized> {
    fd: RawFd,
    _p: PhantomData<T>,
}

impl<T: Send + Clone + Sized> TempFile<T> {
    pub fn create<N: AsRef<str>>(name: N) -> TypedResult<Self> {
        let mem = MemfdOptions::default()
            .close_on_exec(false)
            .allow_sealing(true)
            .create(name)
            .typ(SystemError::Panic)?;
        mem.as_file()
            .set_len(
                size_of::<T>()
                    .try_into()
                    .expect("Could not fit usize into u64"),
            )
            .typ(SystemError::Panic)?;
        mem.add_seals(&[FileSeal::SealShrink, FileSeal::SealGrow])
            .typ(SystemError::Panic)?;

        Ok(Self {
            fd: mem.into_raw_fd(),
            _p: PhantomData,
        })
    }

    fn get_memfd(&self) -> TypedResult<Memfd> {
        let fd = dup(self.fd).typ(SystemError::Panic)?;
        Memfd::try_from_fd(fd)
            .map_err(|e| {
                close(fd).ok();
                anyhow!("Could not get Memfd from {e:#?}")
            })
            .typ(SystemError::Panic)
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

    pub fn seal_read_only(&self) -> TypedResult<TypedMmapMut<T>> {
        let mmap = self.get_typed_mmap_mut()?;

        self.get_memfd()?
            .add_seals(&[FileSeal::SealFutureWrite, FileSeal::SealSeal])
            .typ(SystemError::Panic)?;

        Ok(mmap)
    }

    pub fn fd(&self) -> RawFd {
        self.fd
    }

    pub fn write(&self, value: &T) -> TypedResult<()> {
        let bytes =
            unsafe { std::slice::from_raw_parts(value as *const T as *const u8, size_of::<T>()) };
        let mut file = self.get_memfd()?.into_file();
        file.seek(SeekFrom::Start(0)).typ(SystemError::Panic)?;
        file.write_all(bytes)
            .map_err(anyhow::Error::from)
            .typ(SystemError::Panic)
    }

    pub fn read(&self) -> TypedResult<T> {
        let mut buf = Vec::with_capacity(size_of::<T>());
        let mut file = self.get_memfd()?.into_file();
        file.seek(SeekFrom::Start(0)).typ(SystemError::Panic)?;
        file.read_to_end(buf.as_mut()).typ(SystemError::Panic)?;
        Ok(unsafe { buf.as_slice().align_to::<T>().1[0].clone() })
    }

    pub fn get_typed_mmap_mut(&self) -> TypedResult<TypedMmapMut<T>> {
        let fd = dup(self.fd).typ(SystemError::Panic)?;
        unsafe {
            MmapMut::map_mut(fd)
                .map_err(|e| {
                    close(fd).ok();
                    anyhow!("Could not get Mmap from {e:#?}")
                })
                .typ(SystemError::Panic)?
                .try_into()
        }
    }

    pub fn get_typed_mmap(&self) -> TypedResult<TypedMmap<T>> {
        let fd = dup(self.fd).typ(SystemError::Panic)?;
        unsafe {
            Mmap::map(fd)
                .map_err(|e| {
                    close(fd).ok();
                    anyhow!("Could not get Mmap from {e:#?}")
                })
                .typ(SystemError::Panic)?
                .try_into()
        }
    }
}

impl<T: Send + Clone> TryFrom<RawFd> for TempFile<T> {
    type Error = TypedError;

    fn try_from(fd: RawFd) -> Result<Self, Self::Error> {
        let tf = Self {
            fd,
            _p: PhantomData,
        };
        let memfd = tf.get_memfd()?;
        trace!("Got Memfd from {fd}. Seals: {:?}", memfd.seals());
        Ok(tf)
    }
}

impl<T: Send + Clone + Sized> AsRawFd for TempFile<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

// TODO remove this function
// This may fail if the name is the same as another name or a part of another
pub fn get_memfd(name: &str) -> TypedResult<i32> {
    let name = format!("memfd:{name}");
    Process::myself()
        .typ(SystemError::Panic)?
        .fd()
        .typ(SystemError::Panic)?
        .flatten()
        .find_map(|f| {
            if let FDTarget::Path(p) = &f.target {
                if p.to_str().expect("Got non-UTF-8 String").contains(&name) {
                    Some(f.fd)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow!("No File Descriptor with Name: {name}"))
        .typ(SystemError::Panic)
}
