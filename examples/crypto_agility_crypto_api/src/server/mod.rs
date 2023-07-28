use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use log::{debug, warn};
use num_traits::FromPrimitive;

use crate::{OpCode, ResultCode};

pub mod example;

#[cfg_attr(test, mockall::automock)]
pub trait Endpoint {
    fn signature_key_size(&self) -> usize;
    fn encryption_key_size(&self) -> usize;
    fn get_public_encryption_key(&self) -> Vec<u8>;
    fn get_secret_encryption_key(&self) -> Vec<u8>;
    fn get_public_signature_key(&self) -> Vec<u8>;
    fn get_secret_signature_key(&self) -> Vec<u8>;
    fn sign_and_encrypt(&self, peer_encryption_key: &[u8], message: &[u8]) -> Result<Vec<u8>>;
    fn decrypt_and_verify(
        &self,
        peer_signature_key: &[u8],
        encrypted_message: &[u8],
    ) -> Result<(bool, Vec<u8>)>;
}

pub struct CipherServer<E: Endpoint> {
    endpoints: HashMap<u32, E>,
}

impl<E: Endpoint> Default for CipherServer<E> {
    fn default() -> Self {
        Self {
            endpoints: HashMap::default(),
        }
    }
}

impl<E: Endpoint> CipherServer<E> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_endpoint(&mut self, id: u32, endpoint: E) {
        self.endpoints.insert(id, endpoint);
    }

    fn process_internal(&self, endpoint: u32, opcode: OpCode, payload: &[u8]) -> Result<Vec<u8>> {
        let endpoint = self
            .endpoints
            .get(&endpoint)
            .context(format!("endpoint {endpoint} does not exist"))?;
        match opcode {
            OpCode::Encrypt => Self::process_encrypt(endpoint, payload),
            OpCode::Decrypt => Self::process_decrypt(endpoint, payload),
            OpCode::RequestPeerEncryptionKey => self.process_peer_encryption_key_request(payload),
            OpCode::RequestPeerSignatureKey => self.process_peer_signature_key_request(payload),
        }
    }

    pub fn process_endpoint_request(&self, endpoint: u32, request: &[u8]) -> Vec<u8> {
        let Some((opcode, payload)) = request.split_first() else {
            return Err(anyhow::anyhow!("no opcode specified")).with_result_encoding(0);
        };
        let Some(opcode) = OpCode::from_u8(*opcode) else {
            return Err(anyhow::anyhow!("unexpected opcode: {opcode}"))
                .with_result_encoding(*opcode);
        };
        self.process_internal(endpoint, opcode, payload)
            .with_result_encoding(opcode as u8)
    }

    fn process_encrypt(endpoint: &E, request: &[u8]) -> Result<Vec<u8>> {
        let encryption_key_size = endpoint.encryption_key_size();
        if request.len() < encryption_key_size {
            bail!("encryption request ({}) is smaller than encryption key size ({encryption_key_size})", request.len())
        }
        let (encryption_key, message) = request.split_at(encryption_key_size);
        endpoint.sign_and_encrypt(encryption_key, message)
    }

    fn process_decrypt(endpoint: &E, request: &[u8]) -> Result<Vec<u8>> {
        let signature_key_size = endpoint.signature_key_size();
        if request.len() < signature_key_size {
            bail!(
                "decryption request ({}) is smaller than signature key size ({signature_key_size})",
                request.len()
            )
        }
        let (signature_key, encrypted_message) = request.split_at(signature_key_size);
        let (signed, message) = endpoint.decrypt_and_verify(signature_key, encrypted_message)?;
        let mut result = vec![signed as u8];
        result.extend_from_slice(&message);
        Ok(result)
    }

    fn process_peer_encryption_key_request(&self, request: &[u8]) -> Result<Vec<u8>> {
        debug!("answer public encryption key request");
        let peer_bytes = request.try_into().context("peer request bytes issue")?;
        let peer_id = u32::from_le_bytes(peer_bytes);
        let peer = self
            .endpoints
            .get(&peer_id)
            .context(format!("endpoint {peer_id} does not exist"))?;
        Ok(peer.get_public_encryption_key())
    }

    fn process_peer_signature_key_request(&self, request: &[u8]) -> Result<Vec<u8>> {
        debug!("answer public signature key request");
        let peer_bytes = request.try_into().context("peer request bytes issue")?;
        let peer_id = u32::from_le_bytes(peer_bytes);
        let peer = self
            .endpoints
            .get(&peer_id)
            .context(format!("endpoint {peer_id} does not exist"))?;
        Ok(peer.get_public_signature_key())
    }
}

trait ResultEncodingExt {
    fn with_result_encoding(self, opcode: u8) -> Vec<u8>;
}

impl ResultEncodingExt for Result<Vec<u8>, anyhow::Error> {
    fn with_result_encoding(self, opcode: u8) -> Vec<u8> {
        match self {
            Ok(content) => {
                let mut response = vec![ResultCode::Ok as u8, opcode];
                response.extend_from_slice(&content);
                response
            }
            Err(err) => {
                let mut response = vec![ResultCode::Error as u8, opcode];
                warn!("{err}");
                response.extend_from_slice(&err.to_string().into_bytes());
                response
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cipher_server_process_encrypt() {
        let mut endpoint = MockEndpoint::new();

        // Define the expected behavior for the mock endpoint
        endpoint.expect_encryption_key_size().return_const(1usize);
        endpoint
            .expect_sign_and_encrypt()
            .withf(|public_key, message| public_key == [1] && message == [2, 3])
            .returning(|_, _| Ok(Vec::from("encrypted-data")));

        // Insert the mock endpoint into the CipherServer
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint);

        // Perform the test for OpCode::Encrypt
        let request = vec![OpCode::Encrypt as u8, 1, 2, 3];
        let response = cipher_server.process_endpoint_request(1, &request);

        assert_eq!(response[0], ResultCode::Ok as u8);
        assert_eq!(response[1], OpCode::Encrypt as u8);
        assert_eq!(&response[2..], b"encrypted-data");
    }

    #[test]
    fn test_cipher_server_process_decrypt() {
        let mut endpoint = MockEndpoint::new();

        // Define the expected behavior for the mock endpoint
        endpoint.expect_signature_key_size().return_const(3usize);
        endpoint
            .expect_decrypt_and_verify()
            .withf(|signature, encrypted_message| {
                signature == [4, 5, 6] && encrypted_message == [7, 8, 9]
            })
            .returning(|_, _| Ok((true, Vec::from("decrypted-data"))));

        // Insert the mock endpoint into the CipherServer
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint);

        // Perform the test for OpCode::Decrypt
        let signature = &[4, 5, 6];
        let encrypted_message = &[7, 8, 9];
        let mut request = vec![OpCode::Decrypt as u8];
        request.extend_from_slice(signature);
        request.extend_from_slice(encrypted_message);
        let response = cipher_server.process_endpoint_request(1, &request);

        assert_eq!(response[0], ResultCode::Ok as u8);
        assert_eq!(response[1], OpCode::Decrypt as u8);
        assert!(response[2] > 0);
        assert_eq!(&response[3..], b"decrypted-data");
    }

    #[test]
    fn test_cipher_server_process_request_peer_encryption_key() {
        let endpoint1 = MockEndpoint::new();
        let mut endpoint2 = MockEndpoint::new();

        // Define the expected behavior for the mock endpoints
        endpoint2
            .expect_get_public_encryption_key()
            .return_const(Vec::from("public-encryption-key-2"));

        // Insert the mock endpoints into the CipherServer
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint1);
        cipher_server.insert_endpoint(2, endpoint2);

        // Perform the test to check public encryption keys of both endpoints
        let peer_id = 2_u32.to_le_bytes();
        let request = vec![OpCode::RequestPeerEncryptionKey as u8];
        let request = [request.as_slice(), &peer_id].concat();
        let response = cipher_server.process_endpoint_request(1, &request);

        assert_eq!(response[0], ResultCode::Ok as u8);
        assert_eq!(response[1], OpCode::RequestPeerEncryptionKey as u8);
        assert_eq!(&response[2..], b"public-encryption-key-2");
    }

    #[test]
    fn test_cipher_server_process_request_peer_signature_key() {
        let endpoint1 = MockEndpoint::new();
        let mut endpoint2 = MockEndpoint::new();

        // Define the expected behavior for the mock endpoints
        endpoint2
            .expect_get_public_signature_key()
            .return_const(Vec::from("public-signature-key-2"));

        // Insert the mock endpoints into the CipherServer
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint1);
        cipher_server.insert_endpoint(2, endpoint2);

        // Perform the test to check public signature keys of both endpoints
        let peer_id = 2_u32.to_le_bytes();
        let request = vec![OpCode::RequestPeerSignatureKey as u8];
        let request = [request.as_slice(), &peer_id].concat();
        let response = cipher_server.process_endpoint_request(1, &request);

        assert_eq!(response[0], ResultCode::Ok as u8);
        assert_eq!(response[1], OpCode::RequestPeerSignatureKey as u8);
        assert_eq!(&response[2..], b"public-signature-key-2");
    }

    #[test]
    fn test_cipher_server_process_unknown_opcode() {
        // Insert a mock endpoint into the CipherServer
        let endpoint = MockEndpoint::new();
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint);

        // Perform the test for an unknown opcode
        let request = vec![100, 13, 14, 15];
        let response = cipher_server.process_endpoint_request(1, &request);

        assert_eq!(response[0], ResultCode::Error as u8); // Ensure that an error is returned for an
                                                          // unknown opcode
        assert_eq!(response[1], request[0]);
    }
    #[test]
    fn test_cipher_server_process_encrypt_empty_request_success() {
        let mut endpoint = MockEndpoint::new();

        // Define the expected behavior for the mock endpoint.
        endpoint.expect_encryption_key_size().return_const(0usize);
        endpoint
            .expect_sign_and_encrypt()
            .withf(|public_key, message| public_key.is_empty() && message.is_empty())
            .returning(|_, _| Ok(Vec::from("encrypted-data")));

        // Insert the mock endpoint into the CipherServer
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint);

        // Perform the test for OpCode::Encrypt with an empty request.
        let request = vec![OpCode::Encrypt as u8];
        let response = cipher_server.process_endpoint_request(1, &request);

        assert_eq!(response[0], ResultCode::Ok as u8);
        assert_eq!(response[1], OpCode::Encrypt as u8);
        assert_eq!(&response[2..], b"encrypted-data");
    }

    #[test]
    fn test_cipher_server_process_encrypt_empty_request_fail() {
        let mut endpoint = MockEndpoint::new();

        // Define the expected behavior for the mock endpoint.
        endpoint.expect_encryption_key_size().return_const(0usize);
        endpoint
            .expect_sign_and_encrypt()
            .withf(|public_key, message| public_key.is_empty() && message.is_empty())
            .returning(|_, _| Err(anyhow::anyhow!("empty request not allowed")));

        // Insert the mock endpoint into the CipherServer
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint);

        // Perform the test for OpCode::Encrypt with an empty request.
        let request = vec![OpCode::Encrypt as u8];
        let response = cipher_server.process_endpoint_request(1, &request);

        assert_eq!(response[0], ResultCode::Error as u8); // The test should
                                                          // fail due to the
                                                          // mock
                                                          // behavior.
    }
    #[test]
    fn test_cipher_server_process_decrypt_empty_request() {
        let mut endpoint = MockEndpoint::new();

        // Define the expected behavior for the mock endpoint
        endpoint.expect_signature_key_size().return_const(4usize);

        // Insert the mock endpoint into the CipherServer
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint);

        // Perform the test for OpCode::Decrypt with an empty request
        let request = vec![OpCode::Decrypt as u8];
        let response = cipher_server.process_endpoint_request(1, &request);

        assert_eq!(response[0], ResultCode::Error as u8); // Ensure that an
                                                          // error is returned
                                                          // for an
                                                          // empty request
    }

    #[test]
    fn test_cipher_server_process_decrypt_large_signature_key() {
        let mut endpoint = MockEndpoint::new();

        // Define the expected behavior for the mock endpoint
        endpoint.expect_signature_key_size().return_const(4usize);
        endpoint
            .expect_decrypt_and_verify()
            .returning(|_, _| Ok((true, Vec::from("decrypted-data"))));

        // Insert the mock endpoint into the CipherServer
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint);

        // Perform the test for OpCode::Decrypt with a large signature key
        let signature = &[1, 2, 3, 4, 5]; // Larger than the signature key size
        let encrypted_message = &[6, 7, 8, 9];
        let mut request = vec![OpCode::Decrypt as u8];
        request.extend_from_slice(signature);
        request.extend_from_slice(encrypted_message);
        let response = cipher_server.process_endpoint_request(1, &request);

        assert_eq!(response[0], ResultCode::Ok as u8);
        assert_eq!(response[1], OpCode::Decrypt as u8);
        assert!(response[2] > 0);
        assert_eq!(&response[3..], b"decrypted-data");
    }

    #[test]
    fn test_cipher_server_process_decrypt_exact_signature_key_size() {
        let mut endpoint = MockEndpoint::new();

        // Define the expected behavior for the mock endpoint
        endpoint.expect_signature_key_size().return_const(4usize);
        endpoint
            .expect_decrypt_and_verify()
            .returning(|_, _| Ok((true, Vec::from("decrypted-data"))));

        // Insert the mock endpoint into the CipherServer
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint);

        // Perform the test for OpCode::Decrypt with a signature key of the exact size
        let signature = &[1, 2, 3, 4];
        let encrypted_message = &[6, 7, 8, 9];
        let mut request = vec![OpCode::Decrypt as u8];
        request.extend_from_slice(signature);
        request.extend_from_slice(encrypted_message);
        let response = cipher_server.process_endpoint_request(1, &request);

        assert_eq!(response[0], ResultCode::Ok as u8);
        assert_eq!(response[1], OpCode::Decrypt as u8);
        assert!(response[2] > 0);
        assert_eq!(&response[3..], b"decrypted-data");
    }

    #[test]
    fn test_cipher_server_process_decrypt_empty_encrypted_message_success() {
        let mut endpoint = MockEndpoint::new();

        // Define the expected behavior for the mock endpoint
        endpoint.expect_signature_key_size().return_const(4usize);
        endpoint
            .expect_decrypt_and_verify()
            .returning(|_, _| Ok((true, Vec::from("decrypted-data"))));

        // Insert the mock endpoint into the CipherServer
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint);

        // Perform the test for OpCode::Decrypt with an empty encrypted message.
        let signature_key = &[1, 2, 3, 4];
        let encrypted_message = &[];
        let mut request = vec![OpCode::Decrypt as u8];
        request.extend_from_slice(signature_key);
        request.extend_from_slice(encrypted_message);
        let response = cipher_server.process_endpoint_request(1, &request);

        assert_eq!(response[0], ResultCode::Ok as u8);
        assert_eq!(response[1], OpCode::Decrypt as u8);
        assert!(response[2] > 0);
        assert_eq!(&response[3..], b"decrypted-data");
    }
    #[test]
    fn test_cipher_server_process_decrypt_empty_signature_key() {
        let mut endpoint = MockEndpoint::new();

        // Define the expected behavior for the mock endpoint
        endpoint.expect_signature_key_size().return_const(4usize);
        endpoint
            .expect_decrypt_and_verify()
            .returning(|_, _| Ok((true, Vec::from("decrypted-data"))));

        // Insert the mock endpoint into the CipherServer
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint);

        // Perform the test for OpCode::Decrypt with an empty signature key.
        let signature_key = &[];
        let encrypted_message = &[1, 2, 3, 4];
        let mut request = vec![OpCode::Decrypt as u8];
        request.extend_from_slice(signature_key);
        request.extend_from_slice(encrypted_message);
        let response = cipher_server.process_endpoint_request(1, &request);

        assert_eq!(response[0], ResultCode::Ok as u8);
        assert_eq!(response[1], OpCode::Decrypt as u8);
        assert!(response[2] > 0);
        assert_eq!(&response[3..], b"decrypted-data");
    }
    #[test]
    fn test_cipher_server_process_decrypt_empty_encrypted_message_fail() {
        let mut endpoint = MockEndpoint::new();

        // Define the expected behavior for the mock endpoint
        endpoint.expect_signature_key_size().return_const(4usize);
        endpoint
            .expect_decrypt_and_verify()
            .withf(|_, encrypted_message| encrypted_message.is_empty())
            .returning(|_, _| Err(anyhow::anyhow!("Empty encrypted message")));

        // Insert the mock endpoint into the CipherServer
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint);

        // Perform the test for OpCode::Decrypt with an empty encrypted message.
        let signature_key = &[1, 2, 3, 4];
        let encrypted_message = &[];
        let mut request = vec![OpCode::Decrypt as u8];
        request.extend_from_slice(signature_key);
        request.extend_from_slice(encrypted_message);
        let response = cipher_server.process_endpoint_request(1, &request);

        assert_eq!(response[0], ResultCode::Error as u8); // The test should fail due to the mock
                                                          // returning an error.
        assert_eq!(response[1], OpCode::Decrypt as u8);
    }
}

// Integration Test Module
#[cfg(test)]
mod integration_tests {
    use crate::client::{request::RequestBuilder, response::Response, Key};

    use super::*;

    #[test]
    fn test_cipher_server_encrypt() {
        // Define a mock endpoint for the test
        let mut endpoint = MockEndpoint::new();
        endpoint.expect_encryption_key_size().return_const(4usize);
        endpoint
            .expect_sign_and_encrypt()
            .withf(|public_key, message| public_key == [1, 2, 3, 4] && message == [1, 2, 3])
            .returning(|_, _| Ok(Vec::from("encrypted-data-1")));

        // Create the CipherServer and insert the mock endpoint
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint);

        // Test OpCode::Encrypt
        let message = vec![1, 2, 3];
        let encryption_key: Key<4> = [1, 2, 3, 4].as_slice().try_into().unwrap();
        let mut request_builder_buffer = [0u8; 32768];
        let mut request_builder = RequestBuilder::new(&mut request_builder_buffer);
        let request = request_builder
            .build_encryption_request(&encryption_key, &message)
            .unwrap();
        let response = cipher_server.process_endpoint_request(1, request);
        let response = Response::try_from(response.as_slice()).unwrap();
        if let Response::EncryptedMessage(data) = response {
            assert_eq!(data, b"encrypted-data-1");
        } else {
            panic!("Unexpected response type for OpCode::Encrypt");
        }
    }

    #[test]
    fn test_cipher_server_decrypt() {
        // Define a mock endpoint for the test
        let mut endpoint = MockEndpoint::new();
        endpoint.expect_signature_key_size().return_const(4usize);
        endpoint
            .expect_decrypt_and_verify()
            .withf(|signature, encrypted_message| {
                signature == [1, 2, 3, 4] && encrypted_message == [5, 6, 7]
            })
            .returning(|_, _| Ok((true, Vec::from("decrypted-data"))));

        // Create the CipherServer and insert the mock endpoint
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint);

        // Test OpCode::Decrypt
        let signature: Key<4> = [1, 2, 3, 4].as_slice().try_into().unwrap();
        let encrypted_message = [5, 6, 7];
        let mut request_builder_buffer = [0u8; 32768];
        let mut request_builder = RequestBuilder::new(&mut request_builder_buffer);
        let request = request_builder
            .build_decrypt_request(&signature, &encrypted_message)
            .unwrap();
        let response = cipher_server.process_endpoint_request(1, request);
        let response = Response::try_from(response.as_slice()).unwrap();
        if let Response::DecryptedMessage { signed, message } = response {
            assert!(signed);
            assert_eq!(message, b"decrypted-data");
        } else {
            panic!("Unexpected response type for OpCode::Decrypt");
        }
    }

    #[test]
    fn test_cipher_server_request_peer_encryption_key() {
        // Define mock endpoints for the test
        let endpoint1 = MockEndpoint::new();
        let mut endpoint2 = MockEndpoint::new();
        endpoint2
            .expect_get_public_encryption_key()
            .return_const(Vec::from("public-encryption-key-2"));

        // Create the CipherServer and insert the mock endpoints
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint1);
        cipher_server.insert_endpoint(2, endpoint2);

        // Test OpCode::RequestPeerEncryptionKey
        let mut request_builder_buffer = [0u8; 32768];
        let mut request_builder = RequestBuilder::new(&mut request_builder_buffer);
        let request = request_builder
            .build_peer_public_encryption_key_request(2)
            .unwrap();
        let response = cipher_server.process_endpoint_request(1, request);
        let response = Response::try_from(response.as_slice()).unwrap();
        if let Response::PeerPublicEncryptionKey(key_response) = response {
            assert_eq!(
                key_response.into_key::<64>().unwrap().read(),
                b"public-encryption-key-2"
            );
        } else {
            panic!("Unexpected response type for OpCode::RequestPeerEncryptionKey. {response:?}");
        }
    }

    #[test]
    fn test_cipher_server_request_peer_signature_key() {
        // Define mock endpoints for the test
        let endpoint1 = MockEndpoint::new();
        let mut endpoint2 = MockEndpoint::new();
        endpoint2
            .expect_get_public_signature_key()
            .return_const(Vec::from("public-signature-key-2"));

        // Create the CipherServer and insert the mock endpoints
        let mut cipher_server = CipherServer::new();
        cipher_server.insert_endpoint(1, endpoint1);
        cipher_server.insert_endpoint(2, endpoint2);

        // Test OpCode::RequestPeerSignatureKey
        let mut request_builder_buffer = [0u8; 32768];
        let mut request_builder = RequestBuilder::new(&mut request_builder_buffer);
        let request = request_builder
            .build_peer_public_signature_key_request(2)
            .unwrap();
        let response = cipher_server.process_endpoint_request(1, request);
        let response = Response::try_from(response.as_slice()).unwrap();
        if let Response::PeerPublicSignatureKey(key_response) = response {
            assert_eq!(
                key_response.into_key::<64>().unwrap().read(),
                b"public-signature-key-2"
            );
        } else {
            panic!("Unexpected response type for OpCode::RequestPeerSignatureKey. {response:?}");
        }
    }
}
