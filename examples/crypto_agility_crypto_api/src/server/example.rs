use anyhow::{anyhow, bail, Context, Result};
use hpke::kem::{Kem as KemTrait, PrivateKey, PublicKey};
use hpke::{Deserializable, OpModeR, OpModeS, Serializable};
use log::debug;

use super::Endpoint;

type Kem = hpke::kem::X25519Kyber768Draft00;
type Aead = hpke::aead::ChaCha20Poly1305;
type Kdf = hpke::kdf::HkdfSha384;
type EncappedKey = <Kem as KemTrait>::EncappedKey;

const SECRET_SIGNATURE_KEY_SIZE: usize = pqc_sphincsplus::CRYPTO_SECRETKEYBYTES;
const PUBLIC_SIGNATURE_KEY_SIZE: usize = pqc_sphincsplus::CRYPTO_PUBLICKEYBYTES;
const SIGNATURE_SIZE: usize = pqc_sphincsplus::CRYPTO_BYTES;

pub struct ExampleEndpoint {
    salt: Vec<u8>,
    public_encryption_key: PublicKey,
    secret_encryption_key: PrivateKey,
    public_signature_key: [u8; PUBLIC_SIGNATURE_KEY_SIZE],
    secret_signature_key: [u8; SECRET_SIGNATURE_KEY_SIZE],
}

impl ExampleEndpoint {
    pub fn new(salt: &[u8]) -> Self {
        let (secret_encryption_key, public_encryption_key) =
            Kem::gen_keypair(&mut rand::thread_rng());
        let signature_keypair = pqc_sphincsplus::keypair();
        Self {
            salt: salt.to_vec(),
            secret_encryption_key,
            public_encryption_key,
            secret_signature_key: signature_keypair.secret,
            public_signature_key: signature_keypair.public,
        }
    }
}

impl ExampleEndpoint {
    fn signature_key_pair(&self) -> pqc_sphincsplus::Keypair {
        pqc_sphincsplus::Keypair {
            public: self.public_signature_key,
            secret: self.secret_signature_key,
        }
    }

    fn sign(&self, message: &[u8]) -> Vec<u8> {
        pqc_sphincsplus::sign(&message, &self.signature_key_pair()).to_vec()
    }

    fn encrypt(
        &self,
        message: &[u8],
        public_encryption_key: PublicKey,
    ) -> Result<(EncappedKey, Vec<u8>)> {
        hpke::single_shot_seal::<Aead, Kdf, Kem, _>(
            &OpModeS::Base,
            &public_encryption_key,
            &self.salt,
            &message,
            &[],
            &mut rand::thread_rng(),
        )
        .map_err(|_| anyhow!("encyption failed"))
    }

    fn verify(
        &self,
        message: &[u8],
        signature: [u8; SIGNATURE_SIZE],
        public_signature_key: [u8; PUBLIC_SIGNATURE_KEY_SIZE],
    ) -> bool {
        let keypair = pqc_sphincsplus::Keypair {
            public: public_signature_key,
            secret: [0u8; SECRET_SIGNATURE_KEY_SIZE],
        };
        pqc_sphincsplus::verify(&signature, &message, &keypair).is_ok()
    }

    fn decrypt(&self, encrypted_message: &EncryptedMessage) -> Result<Vec<u8>> {
        hpke::single_shot_open::<Aead, Kdf, Kem>(
            &OpModeR::Base,
            &self.secret_encryption_key,
            &encrypted_message.encapped_key,
            &self.salt,
            &encrypted_message.cipher_text,
            &[],
        )
        .map_err(|_| anyhow!("decryption failed"))
    }
}

impl Endpoint for ExampleEndpoint {
    fn get_public_encryption_key(&self) -> Vec<u8> {
        self.public_encryption_key.to_bytes().to_vec()
    }

    fn get_secret_encryption_key(&self) -> Vec<u8> {
        self.secret_encryption_key.to_bytes().to_vec()
    }

    fn get_public_signature_key(&self) -> Vec<u8> {
        self.public_signature_key.to_vec()
    }

    fn get_secret_signature_key(&self) -> Vec<u8> {
        self.secret_signature_key.to_vec()
    }

    fn sign_and_encrypt(&self, peer_encryption_key: &[u8], message: &[u8]) -> Result<Vec<u8>> {
        debug!("encrypting message with {} bytes", message.len());
        let public_encryption_key = PublicKey::from_bytes(peer_encryption_key)
            .map_err(|_| anyhow!("public key parse failed"))?;
        let signature = self
            .sign(&message)
            .try_into()
            .map_err(|_| anyhow!("signature parse failed"))?;
        let (encapped_key, cipher_text) = self.encrypt(message, public_encryption_key)?;
        Ok(EncryptedMessage {
            signature,
            encapped_key,
            cipher_text,
        }
        .into())
    }

    fn decrypt_and_verify(
        &self,
        peer_signature_key: &[u8],
        encrypted_message: &[u8],
    ) -> Result<(bool, Vec<u8>)> {
        let encrypted_message = EncryptedMessage::try_from(encrypted_message)?;
        debug!(
            "decrypting message with {} bytes",
            encrypted_message.cipher_text.len()
        );
        let decryted_message = self.decrypt(&encrypted_message)?;
        let peer_signature_key = peer_signature_key
            .try_into()
            .context("wrong signature size")?;
        let signed = self.verify(
            &decryted_message,
            encrypted_message.signature,
            peer_signature_key,
        );
        Ok((signed, decryted_message))
    }

    fn signature_key_size(&self) -> usize {
        PUBLIC_SIGNATURE_KEY_SIZE
    }

    fn encryption_key_size(&self) -> usize {
        PublicKey::size()
    }
}

#[derive(Clone)]
pub struct EncryptedMessage {
    pub signature: [u8; SIGNATURE_SIZE],
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
    fn extract_signature(buffer: &[u8]) -> Result<([u8; SIGNATURE_SIZE], &[u8])> {
        if buffer.len() < SIGNATURE_SIZE {
            bail!("got remaining encrypted message with length: {}, expected at least {SIGNATURE_SIZE} for signature", buffer.len());
        }
        let (signature, rest) = buffer.split_at(SIGNATURE_SIZE);
        Ok((signature.try_into()?, rest))
    }

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
        let (signature, rest) = Self::extract_signature(buffer)?;
        let (encapped_key, cipher_text_buf) = Self::extract_encapped_key(rest)?;
        let cipher_text = cipher_text_buf.to_vec();
        Ok(EncryptedMessage {
            signature,
            encapped_key,
            cipher_text,
        })
    }
}

impl From<EncryptedMessage> for Vec<u8> {
    fn from(value: EncryptedMessage) -> Self {
        let encapped_key_size = EncappedKey::size();
        let mut buffer =
            Vec::with_capacity(SIGNATURE_SIZE + encapped_key_size + value.cipher_text.len());
        buffer.extend(value.signature);
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
        let endpoint = ExampleEndpoint::new(&salt);

        let message = b"Hello, World!".to_vec();

        let public_encryption_key = endpoint.get_public_encryption_key();
        let encrypted_message: EncryptedMessage = endpoint
            .sign_and_encrypt(&public_encryption_key, &message)
            .unwrap()
            .as_slice()
            .try_into()
            .unwrap();

        let public_signature_key = endpoint.get_public_signature_key();
        let encrypted_message_buf: Vec<_> = encrypted_message.into();
        let (signed, decrypted_message) = endpoint
            .decrypt_and_verify(&public_signature_key, &encrypted_message_buf)
            .unwrap();

        assert!(signed);
        assert_eq!(decrypted_message, message);
    }

    #[test]
    fn test_encryption_between_endpoints() {
        let salt = [1, 2, 3, 4];
        let sender = ExampleEndpoint::new(&salt);
        let receiver = ExampleEndpoint::new(&salt);

        let message = b"Hello, Receiver!".to_vec();

        let public_encryption_key = receiver.get_public_encryption_key();
        let encrypted_message: EncryptedMessage = sender
            .sign_and_encrypt(&public_encryption_key, &message)
            .unwrap()
            .as_slice()
            .try_into()
            .unwrap();

        let public_signature_key = sender.get_public_signature_key();
        let encrypted_message_buf: Vec<_> = encrypted_message.into();
        let (signed, decrypted_message) = receiver
            .decrypt_and_verify(&public_signature_key, &encrypted_message_buf)
            .unwrap();

        assert!(signed);
        assert_eq!(decrypted_message, message);
    }

    #[test]
    fn test_empty_message() {
        let salt = [1, 2, 3, 4];
        let endpoint = ExampleEndpoint::new(&salt);

        let message: Vec<u8> = Vec::new(); // Empty message

        let public_encryption_key = endpoint.get_public_encryption_key();
        let encrypted_message: EncryptedMessage = endpoint
            .sign_and_encrypt(&public_encryption_key, &message)
            .unwrap()
            .as_slice()
            .try_into()
            .unwrap();

        let public_signature_key = endpoint.get_public_signature_key();
        let encrypted_message_buf: Vec<_> = encrypted_message.into();
        let (signed, decrypted_message) = endpoint
            .decrypt_and_verify(&public_signature_key, &encrypted_message_buf)
            .unwrap();

        assert!(signed);
        assert_eq!(decrypted_message, message);
    }

    #[test]
    fn test_empty_salt() {
        let salt = [];
        let endpoint = ExampleEndpoint::new(&salt);

        let message = b"Hello, World!".to_vec();

        let public_encryption_key = endpoint.get_public_encryption_key();
        let encrypted_message: EncryptedMessage = endpoint
            .sign_and_encrypt(&public_encryption_key, &message)
            .unwrap()
            .as_slice()
            .try_into()
            .unwrap();

        let public_signature_key = endpoint.get_public_signature_key();
        let encrypted_message_buf: Vec<_> = encrypted_message.into();
        let (signed, decrypted_message) = endpoint
            .decrypt_and_verify(&public_signature_key, &encrypted_message_buf)
            .unwrap();

        assert!(signed);
        assert_eq!(decrypted_message, message);
    }

    #[test]
    fn test_different_salts() {
        let sender_salt = [1, 2, 3, 4];
        let receiver_salt = [5, 2, 3, 4];
        let sender = ExampleEndpoint::new(&sender_salt);
        let receiver = ExampleEndpoint::new(&receiver_salt);

        let message = b"Hello, Receiver!".to_vec();

        let public_encryption_key = receiver.get_public_encryption_key();
        let encrypted_message = sender
            .sign_and_encrypt(&public_encryption_key, &message)
            .unwrap();
        let public_signature_key = sender.get_public_signature_key();
        let decrypted_message =
            receiver.decrypt_and_verify(&public_signature_key, &encrypted_message);

        assert!(decrypted_message.is_err());
    }

    #[test]
    fn test_random_invalid_data() {
        // Test with random invalid data (different sizes, unexpected bytes)
        let salt = [1, 2, 3, 4];
        let endpoint = ExampleEndpoint::new(&salt);
        let message = b"Hello, World!".to_vec();

        let public_encryption_key = endpoint.get_public_encryption_key();
        let encrypted_message = endpoint
            .sign_and_encrypt(&public_encryption_key, &message)
            .unwrap();
        let mut invalid_data = encrypted_message;
        // Introduce a random byte at the end of the ciphertext
        invalid_data.push(rand::random::<u8>());

        let public_signature_key = endpoint.get_public_signature_key();
        let decrypted_message = endpoint.decrypt_and_verify(&public_signature_key, &invalid_data);

        assert!(decrypted_message.is_err());
    }

    #[test]
    fn test_invalid_decryption() {
        let salt = [1, 2, 3, 4];
        let endpoint = ExampleEndpoint::new(&salt);
        let message = b"Hello, World!".to_vec();

        let public_encryption_key = endpoint.get_public_encryption_key();
        let encrypted_message = endpoint
            .sign_and_encrypt(&public_encryption_key, &message)
            .unwrap();

        // Use a different endpoint for decryption
        let different_endpoint = ExampleEndpoint::new(&salt);
        let public_signature_key = endpoint.get_public_signature_key();
        let decrypted_message =
            different_endpoint.decrypt_and_verify(&public_signature_key, &encrypted_message);

        assert!(decrypted_message.is_err());
    }

    #[test]
    fn test_encrypted_message_conversion() {
        let salt = [1, 2, 3, 4];
        let endpoint = ExampleEndpoint::new(&salt);
        let message = b"Hello, World!".to_vec();

        let public_encryption_key = endpoint.get_public_encryption_key();
        let encrypted_message_bytes = endpoint
            .sign_and_encrypt(&public_encryption_key, &message)
            .unwrap();
        let encrypted_message: EncryptedMessage =
            encrypted_message_bytes.as_slice().try_into().unwrap();

        let into_bytes_conversion = Vec::<u8>::from(encrypted_message.clone());
        let from_bytes_conversion =
            EncryptedMessage::try_from(into_bytes_conversion.as_slice()).unwrap();
        assert_eq!(from_bytes_conversion, encrypted_message);
    }
}
