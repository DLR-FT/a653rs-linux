//! Common definitions for the execution of system calls

use anyhow::{anyhow, Result};
use byteorder::{NativeEndian, ReadBytesExt, WriteBytesExt};
use enum_primitive::FromPrimitive;

pub const SYSCALL_SOCKET_PATH: &str = "/syscall-a653";

mod ty;

pub use ty::ApexSyscall;

#[derive(Debug, PartialEq)]
pub struct SyscallRequest {
    pub id: ApexSyscall,
    pub params: Vec<u64>,
}

#[derive(Debug, PartialEq)]
pub struct SyscallResponse {
    pub id: ApexSyscall,
    pub status: u64,
}

impl SyscallRequest {
    /// Serializes a SyscallRequest into its binary representation
    ///
    /// The format for serializing a SyscallRequest is defined as follows:
    /// ```text
    /// id [u64]
    /// nparams [u8]
    /// params [u64 * nparams]
    /// ```
    ///
    /// All integers are encoded in native endian.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let mut serialized: Vec<u8> = Vec::new();
        serialized.write_u64::<NativeEndian>(self.id as u64)?;
        serialized.write_u8(self.params.len().try_into()?)?;
        for &param in &self.params {
            serialized.write_u64::<NativeEndian>(param)?;
        }

        Ok(serialized)
    }

    /// Deserializes a serialized SyscallRequest back into its internal
    /// representation
    pub fn deserialize(serialized: &Vec<u8>) -> Result<Self> {
        let mut serialized: &[u8] = serialized;

        let id = ApexSyscall::from_u64(serialized.read_u64::<NativeEndian>()?)
            .ok_or(anyhow!("deserialization of ApexSyscall failed"))?;

        let nparams = serialized.read_u8()?;
        let mut params: Vec<u64> = Vec::with_capacity(nparams as usize);
        for _ in 0..nparams {
            params.push(serialized.read_u64::<NativeEndian>()?);
        }

        Ok(SyscallRequest { id, params })
    }
}

impl SyscallResponse {
    /// Serializes a SyscallResponse into its binary representation
    ///
    /// The format for serializing a SyscallResponse is defined as follows:
    /// ```text
    /// id [u64]
    /// status [u64]
    /// ```
    ///
    /// All integers are encoded in native endian.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let mut serialized: Vec<u8> = Vec::new();
        serialized.write_u64::<NativeEndian>(self.id as u64)?;
        serialized.write_u64::<NativeEndian>(self.status)?;

        Ok(serialized)
    }

    /// Deserializes a serialized SyscallResponse back into its internal
    /// representation
    pub fn deserialize(serialized: &Vec<u8>) -> Result<Self> {
        let mut serialized: &[u8] = serialized;

        let id = ApexSyscall::from_u64(serialized.read_u64::<NativeEndian>()?)
            .ok_or(anyhow!("deserialization of ApexSyscall failed"))?;
        let status = serialized.read_u64::<NativeEndian>()?;

        Ok(SyscallResponse { id, status })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_request() {
        let request = SyscallRequest {
            id: ApexSyscall::Start,
            params: vec![1, 2, 3],
        };
        let serialized = request.serialize().unwrap();
        let mut serialized: &[u8] = &serialized;

        let id = serialized.read_u64::<NativeEndian>().unwrap();
        assert_eq!(id, ApexSyscall::Start as u64);

        let nparams = serialized.read_u8().unwrap();
        assert_eq!(nparams, 3);

        let params = [
            serialized.read_u64::<NativeEndian>().unwrap(),
            serialized.read_u64::<NativeEndian>().unwrap(),
            serialized.read_u64::<NativeEndian>().unwrap(),
        ];
        assert_eq!(params, [1, 2, 3]);
        assert!(serialized.is_empty());
    }

    #[test]
    fn test_serialize_response() {
        let response = SyscallResponse {
            id: ApexSyscall::Start,
            status: 42,
        };
        let serialized = response.serialize().unwrap();
        let mut serialized: &[u8] = &serialized;

        let id = serialized.read_u64::<NativeEndian>().unwrap();
        assert_eq!(id, ApexSyscall::Start as u64);

        let status = serialized.read_u64::<NativeEndian>().unwrap();
        assert_eq!(status, 42);
        assert!(serialized.is_empty());
    }

    #[test]
    fn test_deserialize_request() {
        let request = SyscallRequest {
            id: ApexSyscall::Start,
            params: vec![1, 2, 3],
        };
        let serialized = request.serialize().unwrap();
        let deserialized = SyscallRequest::deserialize(&serialized).unwrap();
        assert_eq!(request, deserialized);
        assert!(!serialized.is_empty());
    }

    #[test]
    fn test_deserialize_response() {
        let response = SyscallResponse {
            id: ApexSyscall::Start,
            status: 42,
        };
        let serialized = response.serialize().unwrap();
        let deserialized = SyscallResponse::deserialize(&serialized).unwrap();
        assert_eq!(response, deserialized);
        assert!(!serialized.is_empty());
    }
}
