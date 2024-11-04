//! Implementation of in-memory files
use std::marker::PhantomData;
use std::mem::{size_of, MaybeUninit};
use std::os::unix::prelude::{AsRawFd, FileExt, IntoRawFd, RawFd};

use anyhow::{anyhow, Context, Result};
use memfd::{FileSeal, Memfd, MemfdOptions};
use memmap2::{Mmap, MmapMut};
use nix::unistd::{close, dup};
use procfs::process::{FDTarget, Process};

use crate::error::{ResultExt, SystemError, TypedError, TypedResult};
use crate::shmem::{TypedMmap, TypedMmapMut};

#[derive(Debug, Clone, Copy)]
/// Internal struct for handling in-memory files
pub struct TempFile<T: Send + Clone + Sized> {
    // TODO: Consider storing a Memfd instead of a RawFd
    fd: RawFd,
    _p: PhantomData<T>,
}

impl<T: Send + Clone + Sized> TempFile<T> {
    /// Creates an in-memory file
    pub fn create<N: AsRef<str>>(name: N) -> TypedResult<Self> {
        trace!("Create TempFile \"{}\"", name.as_ref());
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

    /// Converts a FD to a Memfd without borrowing ownership
    fn get_memfd(&self) -> TypedResult<Memfd> {
        // TODO: The call to dup(2) may be removed, because RawFd has no real ownership
        let fd = dup(self.fd).typ(SystemError::Panic)?;
        Memfd::try_from_fd(fd)
            .map_err(|e| {
                close(fd).ok();
                anyhow!("Could not get Memfd from {e:#?}")
            })
            .typ(SystemError::Panic)
    }

    /// Set the TempFile to read-only (prevents further seal modifications)
    pub fn seal_read_only(&self) -> TypedResult<TypedMmapMut<T>> {
        let mmap = self.get_typed_mmap_mut()?;

        self.get_memfd()?
            .add_seals(&[FileSeal::SealFutureWrite, FileSeal::SealSeal])
            .typ(SystemError::Panic)?;

        Ok(mmap)
    }

    /// Returns the raw FD of the TempFile
    pub fn fd(&self) -> RawFd {
        self.fd
    }

    /// Writes value to the TempFile (overwrites existing data, but does not
    /// clean)
    // TODO: Consider deleting the previous data?
    pub fn write(&self, value: &T) -> TypedResult<()> {
        // TODO: Use an approach without unsafe
        let bytes =
            unsafe { std::slice::from_raw_parts(value as *const T as *const u8, size_of::<T>()) };
        let file = self.get_memfd()?.into_file();
        file.write_all_at(bytes, 0)
            .map_err(anyhow::Error::from)
            .typ(SystemError::Panic)
    }

    /// Returns all of the TempFile's data
    // TODO check if this is used for sampling ports
    pub fn read(&self) -> TypedResult<T> {
        // MaybeUninit ensures we avoid alignment related UB
        let mut data = MaybeUninit::<T>::uninit();
        let bytes_required = size_of::<T>();

        // the mut buf binding to the data in the MaybeUninit allows writing to the type
        // byte-wise
        let buf =
            unsafe { std::slice::from_raw_parts_mut(data.as_mut_ptr() as *mut u8, size_of::<T>()) };

        let file = self.get_memfd()?.into_file();

        // read_at avoids confusion by moving cursors on shared file descriptors
        let bytes_read = file.read_at(buf, 0).typ(SystemError::Panic)?;

        trace!("read {bytes_read} bytes from memfd {}", self.fd);
        if bytes_read != bytes_required {
            warn!(
                "initialized {} ({bytes_required} bytes in size) with {bytes_read} bytes originating from memfd {}",
                std::any::type_name::<T>(), self.fd()
            );
        }

        Ok(unsafe { data.assume_init() })
    }

    /// Returns a mutable memory map from a TempFile
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

    /// Returns a memory map from a TemplFile
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
        .with_context(|| format!("No File Descriptor with Name: {name}"))
        .typ(SystemError::Panic)
}
