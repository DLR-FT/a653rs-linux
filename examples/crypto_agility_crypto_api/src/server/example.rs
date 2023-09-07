use anyhow::{anyhow, bail, Context, Result};
use hpke::kem::Kem as KemTrait;
use hpke::{Deserializable, OpModeR, OpModeS, Serializable};
use log::debug;

use super::Endpoint;

type Kem = hpke::kem::X25519Kyber768Dilithium;
type Aead = hpke::aead::ChaCha20Poly1305;
type Kdf = hpke::kdf::HkdfSha384;
type EncappedKey = <Kem as KemTrait>::EncappedKey;
type PrivateKey = <hpke::kem::X25519Kyber768Dilithium as hpke::Kem>::PrivateKey;
type PublicKey = <hpke::kem::X25519Kyber768Dilithium as hpke::Kem>::PublicKey;

pub struct HpkeEndpoint {
    info: Vec<u8>,
    public_encryption_key: PublicKey,
    secret_encryption_key: PrivateKey,
}

impl HpkeEndpoint {
    pub fn new(info: &[u8]) -> Self {
        let (secret_encryption_key, public_encryption_key) =
            Kem::gen_keypair(&mut rand::thread_rng());
        Self {
            info: info.to_vec(),
            secret_encryption_key,
            public_encryption_key,
        }
    }
}

impl HpkeEndpoint {
    fn inner_seal(
        &self,
        additional_data: &[u8],
        message: &[u8],
        public_encryption_key: PublicKey,
    ) -> Result<(EncappedKey, Vec<u8>)> {
        let mode = OpModeS::Auth((
            self.secret_encryption_key.clone(),
            self.public_encryption_key.clone(),
        ));
        hpke::single_shot_seal::<Aead, Kdf, Kem, _>(
            &mode,
            &public_encryption_key,
            &self.info,
            message,
            additional_data,
            &mut rand::thread_rng(),
        )
        .map_err(|_| anyhow!("encyption failed"))
    }

    fn inner_open(
        &self,
        additional_data: &[u8],
        peer_public_key: PublicKey,
        encrypted_message: &EncryptedMessage,
    ) -> Result<Vec<u8>> {
        let mode = OpModeR::Auth(peer_public_key);
        hpke::single_shot_open::<Aead, Kdf, Kem>(
            &mode,
            &self.secret_encryption_key,
            &encrypted_message.encapped_key,
            &self.info,
            &encrypted_message.cipher_text,
            additional_data,
        )
        .map_err(|_| anyhow!("decryption failed"))
    }
}

impl Endpoint for HpkeEndpoint {
    fn get_public_key(&self) -> Vec<u8> {
        self.public_encryption_key.to_bytes().to_vec()
    }

    fn seal(
        &self,
        additional_data: &[u8],
        peer_public_key: &[u8],
        message: &[u8],
    ) -> Result<Vec<u8>> {
        debug!("encrypting message with {} bytes", message.len());
        let peer_public_key = PublicKey::from_bytes(peer_public_key)
            .map_err(|_| anyhow!("public key parse failed"))?;
        let (encapped_key, cipher_text) =
            self.inner_seal(additional_data, message, peer_public_key)?;
        Ok(EncryptedMessage {
            encapped_key,
            cipher_text,
        }
        .into())
    }

    fn open(
        &self,
        additional_data: &[u8],
        peer_public_key: &[u8],
        encrypted_message: &[u8],
    ) -> Result<Vec<u8>> {
        let encrypted_message = EncryptedMessage::try_from(encrypted_message)?;
        debug!(
            "decrypting message with {} bytes",
            encrypted_message.cipher_text.len()
        );
        let peer_public_key = PublicKey::from_bytes(peer_public_key)
            .map_err(|_| anyhow!("public key parsing failed"))?;
        let decryted_message =
            self.inner_open(additional_data, peer_public_key, &encrypted_message)?;
        Ok(decryted_message)
    }

    fn public_key_size(&self) -> usize {
        PublicKey::size()
    }
}

#[derive(Clone)]
pub struct EncryptedMessage {
    pub encapped_key: EncappedKey,
    pub cipher_text: Vec<u8>,
}

impl PartialEq for EncryptedMessage {
    fn eq(&self, other: &Self) -> bool {
        if !self.cipher_text.eq(&other.cipher_text) {
            return false;
        }
        self.encapped_key
            .to_bytes()
            .eq(&other.encapped_key.to_bytes())
    }
}

impl std::fmt::Debug for EncryptedMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedMessage")
            .field("encapped_key", &"<Secret>".to_string())
            .field("cipher_text", &self.cipher_text)
            .finish()
    }
}

impl EncryptedMessage {
    fn extract_encapped_key(buffer: &[u8]) -> Result<(EncappedKey, &[u8])> {
        let encapped_key_size = EncappedKey::size();
        if buffer.len() < encapped_key_size {
            bail!(
                "got remaining encrypted message with length: {}, expected at least {encapped_key_size} for encapped key",
                buffer.len(),
            );
        }
        let (encapped_key_buf, rest) = buffer.split_at(encapped_key_size);
        let encapped_key = EncappedKey::from_bytes(encapped_key_buf)
            .ok()
            .context("encapped key parsing failed")?;
        Ok((encapped_key, rest))
    }
}

impl TryFrom<&[u8]> for EncryptedMessage {
    type Error = anyhow::Error;

    fn try_from(buffer: &[u8]) -> Result<Self, Self::Error> {
        let (encapped_key, cipher_text_buf) = Self::extract_encapped_key(buffer)?;
        let cipher_text = cipher_text_buf.to_vec();
        Ok(EncryptedMessage {
            encapped_key,
            cipher_text,
        })
    }
}

impl From<EncryptedMessage> for Vec<u8> {
    fn from(value: EncryptedMessage) -> Self {
        let encapped_key_size = EncappedKey::size();
        let mut buffer = Vec::with_capacity(encapped_key_size + value.cipher_text.len());
        buffer.extend(value.encapped_key.to_bytes());
        buffer.extend(value.cipher_text);
        buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt() {
        let salt = [1, 2, 3, 4];
        let endpoint = HpkeEndpoint::new(&salt);

        let additional_data = b"Foo".to_vec();
        let message = b"Hello, World!".to_vec();

        let public_encryption_key = endpoint.get_public_key();
        let encrypted_message: EncryptedMessage = endpoint
            .seal(&additional_data, &public_encryption_key, &message)
            .unwrap()
            .as_slice()
            .try_into()
            .unwrap();

        let public_key = endpoint.get_public_key();
        let encrypted_message_buf: Vec<_> = encrypted_message.into();
        let decrypted_message = endpoint
            .open(&additional_data, &public_key, &encrypted_message_buf)
            .unwrap();

        assert_eq!(decrypted_message, message);
    }

    #[test]
    fn test_encryption_between_endpoints() {
        let salt = [1, 2, 3, 4];
        let sender = HpkeEndpoint::new(&salt);
        let receiver = HpkeEndpoint::new(&salt);

        let additional_data = b"Foo".to_vec();
        let message = b"Hello, Receiver!".to_vec();

        let public_encryption_key = receiver.get_public_key();
        let encrypted_message: EncryptedMessage = sender
            .seal(&additional_data, &public_encryption_key, &message)
            .unwrap()
            .as_slice()
            .try_into()
            .unwrap();

        let public_key = sender.get_public_key();
        let encrypted_message_buf: Vec<_> = encrypted_message.into();
        let decrypted_message = receiver
            .open(&additional_data, &public_key, &encrypted_message_buf)
            .unwrap();

        assert_eq!(decrypted_message, message);
    }

    #[test]
    fn test_empty_message() {
        let salt = [1, 2, 3, 4];
        let endpoint = HpkeEndpoint::new(&salt);

        let additional_data = b"Foo".to_vec();
        let message: Vec<u8> = Vec::new(); // Empty message

        let public_encryption_key = endpoint.get_public_key();
        let encrypted_message: EncryptedMessage = endpoint
            .seal(&additional_data, &public_encryption_key, &message)
            .unwrap()
            .as_slice()
            .try_into()
            .unwrap();

        let public_key = endpoint.get_public_key();
        let encrypted_message_buf: Vec<_> = encrypted_message.into();
        let decrypted_message = endpoint
            .open(&additional_data, &public_key, &encrypted_message_buf)
            .unwrap();

        assert_eq!(decrypted_message, message);
    }

    #[test]
    fn test_empty_salt() {
        let salt = [];
        let endpoint = HpkeEndpoint::new(&salt);

        let additional_data = b"Foo".to_vec();
        let message = b"Hello, World!".to_vec();

        let public_encryption_key = endpoint.get_public_key();
        let encrypted_message: EncryptedMessage = endpoint
            .seal(&additional_data, &public_encryption_key, &message)
            .unwrap()
            .as_slice()
            .try_into()
            .unwrap();

        let public_key = endpoint.get_public_key();
        let encrypted_message_buf: Vec<_> = encrypted_message.into();
        let decrypted_message = endpoint
            .open(&additional_data, &public_key, &encrypted_message_buf)
            .unwrap();

        assert_eq!(decrypted_message, message);
    }

    #[test]
    fn test_different_salts() {
        let sender_salt = [1, 2, 3, 4];
        let receiver_salt = [5, 2, 3, 4];
        let sender = HpkeEndpoint::new(&sender_salt);
        let receiver = HpkeEndpoint::new(&receiver_salt);

        let additional_data = b"Foo".to_vec();
        let message = b"Hello, Receiver!".to_vec();

        let public_encryption_key = receiver.get_public_key();
        let encrypted_message = sender
            .seal(&additional_data, &public_encryption_key, &message)
            .unwrap();
        let public_key = sender.get_public_key();
        let decrypted_message = receiver.open(&additional_data, &public_key, &encrypted_message);

        assert!(decrypted_message.is_err());
    }

    #[test]
    fn test_random_invalid_data() {
        // Test with random invalid data (different sizes, unexpected bytes)
        let salt = [1, 2, 3, 4];
        let endpoint = HpkeEndpoint::new(&salt);

        let additional_data = b"Foo".to_vec();
        let message = b"Hello, World!".to_vec();

        let public_encryption_key = endpoint.get_public_key();
        let encrypted_message = endpoint
            .seal(&additional_data, &public_encryption_key, &message)
            .unwrap();
        let mut invalid_data = encrypted_message;
        // Introduce a random byte at the end of the ciphertext
        invalid_data.push(rand::random::<u8>());

        let public_key = endpoint.get_public_key();
        let decrypted_message = endpoint.open(&additional_data, &public_key, &invalid_data);

        assert!(decrypted_message.is_err());
    }

    #[test]
    fn test_invalid_decryption() {
        let salt = [1, 2, 3, 4];
        let endpoint = HpkeEndpoint::new(&salt);

        let additional_data = b"Foo".to_vec();
        let message = b"Hello, World!".to_vec();

        let public_encryption_key = endpoint.get_public_key();
        let encrypted_message = endpoint
            .seal(&additional_data, &public_encryption_key, &message)
            .unwrap();

        // Use a different endpoint for decryption
        let different_endpoint = HpkeEndpoint::new(&salt);
        let public_key = endpoint.get_public_key();
        let decrypted_message =
            different_endpoint.open(&additional_data, &public_key, &encrypted_message);

        assert!(decrypted_message.is_err());
    }

    #[test]
    fn test_encrypted_message_conversion() {
        let salt = [1, 2, 3, 4];
        let endpoint = HpkeEndpoint::new(&salt);

        let additional_data = b"Foo".to_vec();
        let message = b"Hello, World!".to_vec();

        let public_encryption_key = endpoint.get_public_key();
        let encrypted_message_bytes = endpoint
            .seal(&additional_data, &public_encryption_key, &message)
            .unwrap();
        let encrypted_message: EncryptedMessage =
            encrypted_message_bytes.as_slice().try_into().unwrap();

        let into_bytes_conversion = Vec::<u8>::from(encrypted_message.clone());
        let from_bytes_conversion =
            EncryptedMessage::try_from(into_bytes_conversion.as_slice()).unwrap();
        assert_eq!(from_bytes_conversion, encrypted_message);
    }
}
