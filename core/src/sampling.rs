use std::collections::HashSet;
use std::convert::AsRef;
use std::os::unix::prelude::{AsRawFd, OwnedFd, RawFd};
use std::time::Instant;

use a653rs::bindings::PortDirection;
use memfd::{FileSeal, Memfd, MemfdOptions};
use memmap2::{Mmap, MmapMut};

use crate::channel::{PortConfig, SamplingChannelConfig};
use crate::error::{ResultExt, SystemError, TypedError, TypedResult};
use crate::partition::SamplingConstant;

#[derive(Debug, Clone)]
struct Datagram<'a> {
    copied: Instant,
    //_len: u32,
    data: &'a [u8], //data: Vec<u8>,
}

impl<'a> Datagram<'a> {
    const EXTRA_BYTES: usize = std::mem::size_of::<Instant>() + std::mem::size_of::<u32>();

    const fn size(msg_size: usize) -> u32 {
        (msg_size + Self::EXTRA_BYTES) as u32
    }

    fn read(mmap: &Mmap, buf: &'a mut [u8]) -> Datagram<'a> {
        loop {
            let (copied_u8, rest) = mmap.as_ref().split_at(std::mem::size_of::<Instant>());
            let (len_u8, data_u8) = rest.split_at(std::mem::size_of::<u32>());

            let copied = unsafe { *(copied_u8.as_ptr() as *const Instant).as_ref().unwrap() };
            let len = unsafe { *(len_u8.as_ptr() as *const u32).as_ref().unwrap() };

            let len = std::cmp::min(len as usize, std::cmp::min(data_u8.len(), buf.len()));
            buf[..len].copy_from_slice(&data_u8[..len]);

            // Make sure that the underlying value didn't change
            let check = unsafe { *(copied_u8.as_ptr() as *const Instant).as_ref().unwrap() };
            if copied == check {
                return Datagram {
                    copied,
                    //_len: len as u32,
                    data: &buf[..len],
                };
            }
        }
    }

    fn write(mmap: &mut MmapMut, write: &[u8]) -> usize {
        let (copied_u8, rest) = mmap.as_mut().split_at_mut(std::mem::size_of::<Instant>());
        let (len_u8, data_u8) = rest.split_at_mut(std::mem::size_of::<u32>());

        let mut_len = unsafe { (len_u8.as_mut_ptr() as *mut u32).as_mut().unwrap() };
        let len = std::cmp::min(data_u8.len(), write.len());
        *mut_len = len as u32;

        data_u8[..len].copy_from_slice(&write[..len]);

        let mut_copied = unsafe { (copied_u8.as_mut_ptr() as *mut Instant).as_mut().unwrap() };
        *mut_copied = Instant::now();

        len
    }
}

#[derive(Debug)]
pub struct Sampling {
    msg_size: usize,
    source_receiver: Mmap,
    source: OwnedFd,
    source_port: PortConfig,
    last: Instant,
    destination_sender: MmapMut,
    destination: OwnedFd,
    destination_ports: HashSet<PortConfig>,
}

impl TryFrom<SamplingChannelConfig> for Sampling {
    type Error = TypedError;

    fn try_from(config: SamplingChannelConfig) -> TypedResult<Self> {
        let msg_size = config.msg_size.as_u64() as usize;
        let source_port_name = config.source.name();
        let (source_receiver, source) =
            Self::source(format!("sampling_{source_port_name}_source"), msg_size)?;
        let (destination_sender, destination) =
            Self::destination(format!("sampling_{source_port_name}_destination"), msg_size)?;

        Ok(Self {
            msg_size,
            source,
            source_receiver,
            source_port: config.source,
            last: Instant::now(),
            destination,
            destination_sender,
            destination_ports: config.destination,
        })
    }
}

impl Sampling {
    pub fn constant<T: AsRef<str>>(&self, part: T) -> Option<SamplingConstant> {
        let (dir, fd, port) = if self.source_port.partition.eq(part.as_ref()) {
            (
                PortDirection::Source,
                self.source_fd(),
                &self.source_port.port,
            )
        } else if let Some(port) = self
            .destination_ports
            .iter()
            .find(|port| port.partition == part.as_ref())
        {
            (
                PortDirection::Destination,
                self.destination_fd(),
                &port.port,
            )
        } else {
            return None;
        };

        Some(SamplingConstant {
            name: port.clone(),
            dir,
            msg_size: self.msg_size,
            fd,
        })
    }

    pub fn name(&self) -> String {
        format!("{}:{}", &self.source_port.partition, &self.source_port.port)
    }

    fn memfd<T: AsRef<str>>(name: T, msg_size: usize) -> TypedResult<Memfd> {
        let size = Datagram::size(msg_size);

        let mem = MemfdOptions::default()
            .close_on_exec(false)
            .allow_sealing(true)
            .create(name)
            .typ(SystemError::Panic)?;
        mem.as_file().set_len(size as u64).typ(SystemError::Panic)?;
        mem.add_seals(&[FileSeal::SealShrink, FileSeal::SealGrow])
            .typ(SystemError::Panic)?;

        Ok(mem)
    }

    fn source<T: AsRef<str>>(name: T, msg_size: usize) -> TypedResult<(Mmap, OwnedFd)> {
        let mem = Self::memfd(name, msg_size)?;

        let mmap = unsafe { Mmap::map(mem.as_raw_fd()).typ(SystemError::Panic)? };

        mem.add_seals(&[FileSeal::SealSeal])
            .typ(SystemError::Panic)?;

        Ok((mmap, mem.into_file().into()))
    }

    fn destination<T: AsRef<str>>(name: T, msg_size: usize) -> TypedResult<(MmapMut, OwnedFd)> {
        let mem = Self::memfd(name, msg_size)?;

        let mmap = unsafe { MmapMut::map_mut(mem.as_raw_fd()).typ(SystemError::Panic)? };

        mem.add_seals(&[FileSeal::SealFutureWrite, FileSeal::SealSeal])
            .typ(SystemError::Panic)?;

        Ok((mmap, mem.into_file().into()))
    }

    //// Returns whether a swap was performed or not
    pub fn swap(&mut self) -> bool {
        let mut buf = vec![0; self.msg_size];
        let read = Datagram::read(&self.source_receiver, &mut buf);
        if self.last == read.copied {
            return false;
        }
        self.last = read.copied;

        Datagram::write(&mut self.destination_sender, read.data);
        true
    }

    pub fn replace_source(&mut self) -> TypedResult<()> {
        let (source_receiver, source) = Self::source(
            format!("sampling_{}_source", self.source_port.port),
            self.msg_size,
        )?;

        self.source = source;
        self.source_receiver = source_receiver;

        Ok(())
    }

    pub fn source_fd(&self) -> RawFd {
        self.source.as_raw_fd()
    }

    pub fn destination_fd(&self) -> RawFd {
        self.destination.as_raw_fd()
    }
}

#[derive(Debug)]
pub struct SamplingSource(MmapMut);

impl SamplingSource {
    pub fn write(&mut self, data: &[u8]) -> usize {
        Datagram::write(&mut self.0, data)
    }
}

impl TryFrom<RawFd> for SamplingSource {
    type Error = TypedError;

    fn try_from(file: RawFd) -> Result<Self, Self::Error> {
        let mmap = unsafe { MmapMut::map_mut(file).typ(SystemError::Panic)? };

        Ok(Self(mmap))
    }
}

#[derive(Debug)]
pub struct SamplingDestination(Mmap);

impl SamplingDestination {
    pub fn read(&mut self, data: &mut [u8]) -> (usize, Instant) {
        let dat = Datagram::read(&self.0, data);

        (dat.data.len(), dat.copied)
    }
}

impl TryFrom<RawFd> for SamplingDestination {
    type Error = TypedError;

    fn try_from(file: RawFd) -> Result<Self, Self::Error> {
        let mmap = unsafe { Mmap::map(file).typ(SystemError::Panic)? };

        Ok(Self(mmap))
    }
}
