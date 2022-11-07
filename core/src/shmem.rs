//! Implementation for shared memory
use std::marker::PhantomData;
use std::mem::size_of;

use anyhow::anyhow;
use memmap2::{Mmap, MmapMut};

use crate::error::{ResultExt, SystemError, TypedError, TypedResult};

#[derive(Debug)]
/// Internal data type for a mutable typed memory map
pub struct TypedMmapMut<T: Send + Sized> {
    mmap: MmapMut,
    _p: PhantomData<T>,
}

impl<T: Send + Sized> TypedMmapMut<T> {
    /// Returns the length of the memory map
    pub fn len(&self) -> usize {
        self.mmap.len()
    }

    /// Checks if the memory map is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Send + Sized> AsRef<T> for TypedMmapMut<T> {
    fn as_ref(&self) -> &T {
        unsafe { (self.mmap.as_ptr() as *const T).as_ref() }.unwrap()
    }
}

impl<T: Send + Sized> AsMut<T> for TypedMmapMut<T> {
    fn as_mut(&mut self) -> &mut T {
        unsafe { (self.mmap.as_mut_ptr() as *mut T).as_mut() }.unwrap()
    }
}

impl<T: Send + Sized> TryFrom<MmapMut> for TypedMmapMut<T> {
    type Error = TypedError;

    fn try_from(mmap: MmapMut) -> TypedResult<Self> {
        let t_size = size_of::<T>();
        let is_size = mmap.len();

        if is_size != t_size {
            return Err(anyhow!("Size mismatch! Expected: {t_size}, Is: {is_size}"))
                .typ(SystemError::Panic);
        }

        Ok(Self {
            mmap,
            _p: PhantomData,
        })
    }
}

#[derive(Debug)]
/// Internal data type for a mutable typed memory map
pub struct TypedMmap<T: Send + Sized> {
    mmap: Mmap,
    _p: PhantomData<T>,
}

impl<T: Send + Sized> TypedMmap<T> {
    /// Returns the length of the memory map
    pub fn len(&self) -> usize {
        self.mmap.len()
    }

    /// Checks if the memory map is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Send + Sized> AsRef<T> for TypedMmap<T> {
    fn as_ref(&self) -> &T {
        unsafe { (self.mmap.as_ptr() as *const T).as_ref() }.unwrap()
    }
}

impl<T: Send + Sized> TryFrom<Mmap> for TypedMmap<T> {
    type Error = TypedError;

    fn try_from(mmap: Mmap) -> TypedResult<Self> {
        let t_size = size_of::<T>();
        let is_size = mmap.len();

        if is_size != t_size {
            return Err(anyhow!("Size mismatch! Expected: {t_size}, Is: {is_size}"))
                .typ(SystemError::Panic);
        }

        Ok(Self {
            mmap,
            _p: PhantomData,
        })
    }
}
