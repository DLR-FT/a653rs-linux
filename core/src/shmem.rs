use anyhow::{anyhow, Result};
use bytemuck::Pod;
use memmap2::{Mmap, MmapMut};

pub trait MmapMutExt {
    fn as_mut_type<T: Pod>(&mut self) -> Result<&mut T>;
}

impl MmapMutExt for MmapMut {
    fn as_mut_type<T: Pod>(&mut self) -> Result<&mut T> {
        bytemuck::try_from_bytes_mut(self.as_mut()).map_err(|e| anyhow!("{e:#?}"))
    }
}

pub trait MmapExt {
    fn as_ref_type<T: Pod>(&self) -> Result<&T>;
}

impl MmapExt for MmapMut {
    fn as_ref_type<T: Pod>(&self) -> Result<&T> {
        bytemuck::try_from_bytes(self.as_ref()).map_err(|e| anyhow!("{e:#?}"))
    }
}

impl MmapExt for Mmap {
    fn as_ref_type<T: Pod>(&self) -> Result<&T> {
        bytemuck::try_from_bytes(self.as_ref()).map_err(|e| anyhow!("{e:#?}"))
    }
}
