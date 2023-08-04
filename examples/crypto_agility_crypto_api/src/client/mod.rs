pub mod request;
pub mod response;

#[derive(Debug, Clone, Copy)]
pub enum Error {
    OutOfSpace,
}

#[derive(Debug, Clone)]
pub struct Key<const MAX_KEYSIZE: usize> {
    key: heapless::Vec<u8, MAX_KEYSIZE>,
}

impl<const MAX_KEYSIZE: usize> Key<MAX_KEYSIZE> {
    pub fn len(&self) -> usize {
        self.key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn read(&self) -> &[u8] {
        &self.key
    }
}

impl<const MAX_KEY_SIZE: usize> TryFrom<&[u8]> for Key<MAX_KEY_SIZE> {
    type Error = Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self {
            key: heapless::Vec::from_slice(value).map_err(|_| Error::OutOfSpace)?,
        })
    }
}
