//! Implementation of a memory fd

use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, RawFd};

use anyhow::{bail, Result};
use memfd::{FileSeal, Memfd, MemfdOptions};

pub struct Mfd(Memfd);

pub enum Seals {
    /// No seals are placed
    Unsealed,
    /// SealShrink + SealGrow + SealWrite + SealSeal
    Readable,
    /// SealSeal
    Writable,
}

impl Mfd {
    /// Creates an empty named Memfd
    pub fn create(name: &str) -> Result<Self> {
        let opts = MemfdOptions::default().allow_sealing(true);
        let mfd = opts.create(name)?;
        Ok(Self(mfd))
    }

    /// Reads all data available
    pub fn read_all(&mut self) -> Result<Vec<u8>> {
        self.0.as_file().seek(SeekFrom::Start(0))?;
        // TODO: Evaluate whether inlining ofsmall message directly into the dtagram
        // makes sense.
        let mut buf: Vec<u8> = Vec::new();
        self.0.as_file().read_to_end(&mut buf)?;

        Ok(buf)
    }

    /// Wipes the mfd and overwrites it
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.0.as_file().seek(SeekFrom::Start(0))?;
        self.0.as_file().set_len(0)?;
        self.0.as_file().write_all(data)?;
        Ok(())
    }

    /// Finalizes the mfd so that it becomes immutable
    pub fn finalize(&mut self, seals: Seals) -> Result<()> {
        let file_seals: Vec<FileSeal> = match seals {
            Seals::Unsealed => vec![],
            Seals::Readable => vec![
                FileSeal::SealShrink,
                FileSeal::SealGrow,
                FileSeal::SealWrite,
                FileSeal::SealSeal,
            ],
            Seals::Writable => vec![FileSeal::SealSeal],
        };

        for seal in file_seals {
            self.0.add_seal(seal)?;
        }

        Ok(())
    }

    /// Returns the actual FD behind the mfd
    /// TODO: Implement the AsRawFd trait instead
    pub fn get_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }

    /// Creates a memfd from a fd
    // TODO: Use some Rust try_from stuff
    pub fn from_fd(fd: RawFd) -> Result<Self> {
        let fd = match Memfd::try_from_fd(fd) {
            Ok(memfd) => memfd,
            Err(_) => bail!("cannot get Memfd from RawFd"),
        };
        Ok(Self(fd))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mfd() {
        let mut mfd = Mfd::create("test").unwrap();
        mfd.write("Hello, world!".as_bytes()).unwrap();
        mfd.finalize(Seals::Readable).unwrap();

        assert_eq!(mfd.read_all().unwrap(), "Hello, world!".as_bytes());
    }
}
