use std::fmt::Debug;
use std::mem::size_of;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use raw_sync::locks::*;
use shared_memory::{Shmem, ShmemConf};

pub struct SamplingPort {
    shm: PathBuf,
    _drop: Shmem,
}

impl Debug for SamplingPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SamplingPort")
            .field("shm", &self.shm)
            .finish()
    }
}

pub struct SamplingPortSource {
    #[allow(dead_code)]
    shm: Shmem,
}

pub struct SamplingPortDestination {
    #[allow(dead_code)]
    shm: Shmem,
}

/// TODO: Use a double buffered sharedmemory approach instead
impl SamplingPort {
    pub(crate) fn new<P: AsRef<Path>>(path: P, size: usize) -> Result<Self> {
        let mutex_reserved = size_of::<libc::pthread_mutex_t>();
        let shm = ShmemConf::new()
            .size(mutex_reserved + size)
            .flink(&path)
            .create()?;
        unsafe {
            RwLock::new(
                shm.as_ptr(),                     // Base address of Mutex
                shm.as_ptr().add(mutex_reserved), // Address of data protected by mutex
            )
            .map_err(|e| anyhow!("{e:#?}"))?;
        }
        Ok(SamplingPort {
            shm: PathBuf::from(path.as_ref()),
            _drop: shm,
        })
    }
}
