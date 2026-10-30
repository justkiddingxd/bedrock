//! End-to-end encryption.
//!
//! Each vault has a random 256-bit vault key. It is stored on the server
//! wrapped (XChaCha20-Poly1305) with a key derived from the vault password
//! (Argon2id), so the server never sees it, and changing the password would
//! only re-wrap it. Two subkeys are derived from it with HKDF-SHA256:
//!
//! * `enc` encrypts file blobs: `version || nonce(24) || ciphertext`, where the
//!   plaintext is `path_len(u32 LE) || path || content` and the AAD binds the
//!   blob to its vault and file id, so the server cannot swap blobs around.
//! * `ids` computes file ids as HMAC-SHA256(path), so the server sees neither
//!   names nor folder structure.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use bedrock_proto::{plain, KeyEnvelope};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const ENVELOPE_AAD: &[u8] = b"bedrock-vault-key-v1";
const BLOB_VERSION: u8 = 1;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("неверный пароль")]
    WrongPassword,
    #[error("повреждённые данные: {0}")]
    Corrupt(&'static str),
    #[error("параметры KDF: {0}")]
    Kdf(String),
}

/// Argon2id cost parameters (memory in KiB).
#[derive(Debug, Clone, Copy)]
pub struct KdfParams {
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl KdfParams {
    /// 64 MiB, 3 passes: about a second on a laptop.
    pub const DEFAULT: Self = Self { m_cost: 64 * 1024, t_cost: 3, p_cost: 1 };
    /// Cheap parameters for tests only.
    pub const INSECURE_TEST: Self = Self { m_cost: 1024, t_cost: 1, p_cost: 1 };
}

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct VaultKey {
    raw: [u8; 32],
    enc: [u8; 32],
    ids: [u8; 32],
}

impl VaultKey {
    pub fn generate() -> Self {
        let mut raw = [0u8; 32];
        OsRng.fill_bytes(&mut raw);
        Self::from_raw(raw)
    }

    pub fn from_raw(raw: [u8; 32]) -> Self {
        let hk = Hkdf::<Sha256>::new(None, &raw);
        let mut enc = [0u8; 32];
        let mut ids = [0u8; 32];
        hk.expand(b"bedrock file encryption v1", &mut enc).expect("valid HKDF length");
        hk.expand(b"bedrock file ids v1", &mut ids).expect("valid HKDF length");
        Self { raw, enc, ids }
    }

    pub fn to_base64(&self) -> String {
        B64.encode(self.raw)
    }

    pub fn from_base64(s: &str) -> Result<Self, CryptoError> {
        let bytes = Zeroizing::new(B64.decode(s).map_err(|_| CryptoError::Corrupt("ключ не в base64"))?);
        let raw: [u8; 32] = bytes.as_slice().try_into().map_err(|_| CryptoError::Corrupt("длина ключа"))?;
        Ok(Self::from_raw(raw))
    }

    /// Opaque, deterministic id of a vault path (64 hex chars).
    pub fn file_id(&self, path: &str) -> String {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.ids).expect("HMAC accepts any key length");
        mac.update(path.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    pub fn seal(&self, vault_id: &str, file_id: &str, path: &str, content: &[u8]) -> Vec<u8> {
        let cipher = <XChaCha20Poly1305 as KeyInit>::new_from_slice(&self.enc).expect("32-byte key");
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);

        let mut plaintext = Zeroizing::new(Vec::with_capacity(4 + path.len() + content.len()));
        plaintext.extend_from_slice(&(path.len() as u32).to_le_bytes());
        plaintext.extend_from_slice(path.as_bytes());
        plaintext.extend_from_slice(content);

        let aad = blob_aad(vault_id, file_id);
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: &plaintext, aad: &aad })
            .expect("XChaCha20-Poly1305 encryption does not fail");

        let mut out = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        out.push(BLOB_VERSION);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        out
    }

    /// Decrypts a blob and returns `(path, content)`.
    pub fn open(&self, vault_id: &str, file_id: &str, blob: &[u8]) -> Result<(String, Vec<u8>), CryptoError> {
        if blob.len() < 1 + NONCE_LEN + TAG_LEN {
            return Err(CryptoError::Corrupt("слишком короткий блоб"));
        }
        if blob[0] != BLOB_VERSION {
            return Err(CryptoError::Corrupt("неизвестная версия блоба"));
        }
        let cipher = <XChaCha20Poly1305 as KeyInit>::new_from_slice(&self.enc).expect("32-byte key");
        let aad = blob_aad(vault_id, file_id);
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(XNonce::from_slice(&blob[1..1 + NONCE_LEN]), Payload { msg: &blob[1 + NONCE_LEN..], aad: &aad })
                .map_err(|_| CryptoError::Corrupt("не удалось расшифровать"))?,
        );
        if plaintext.len() < 4 {
            return Err(CryptoError::Corrupt("нет заголовка"));
        }
        let path_len = u32::from_le_bytes(plaintext[..4].try_into().unwrap()) as usize;
        if plaintext.len() < 4 + path_len {
            return Err(CryptoError::Corrupt("длина пути"));
        }
        let path = std::str::from_utf8(&plaintext[4..4 + path_len])
            .map_err(|_| CryptoError::Corrupt("путь не UTF-8"))?
            .to_string();
        if self.file_id(&path) != file_id {
            return Err(CryptoError::Corrupt("file_id не совпадает с путём"));
        }
        Ok((path, plaintext[4 + path_len..].to_vec()))
    }
}

/// How a vault's blobs are written: encrypted for `e2e` vaults, in the open
/// format the server can read for `server` vaults.
#[derive(Clone)]
pub enum Cipher {
    E2e(VaultKey),
    Plain,
}

impl Cipher {
    pub fn file_id(&self, path: &str) -> String {
        match self {
            Cipher::E2e(key) => key.file_id(path),
            Cipher::Plain => plain::file_id(path),
        }
    }

    pub fn seal(&self, vault_id: &str, file_id: &str, path: &str, content: &[u8]) -> Vec<u8> {
        match self {
            Cipher::E2e(key) => key.seal(vault_id, file_id, path, content),
            Cipher::Plain => plain::encode(path, content),
        }
    }

    pub fn open(&self, vault_id: &str, file_id: &str, blob: &[u8]) -> Result<(String, Vec<u8>), CryptoError> {
        match self {
            Cipher::E2e(key) => key.open(vault_id, file_id, blob),
            Cipher::Plain => {
                let (path, content) = plain::decode(blob).ok_or(CryptoError::Corrupt("неверный формат блоба"))?;
                if plain::file_id(&path) != file_id {
                    return Err(CryptoError::Corrupt("file_id не совпадает с путём"));
                }
                Ok((path, content))
            }
        }
    }
}

fn blob_aad(vault_id: &str, file_id: &str) -> Vec<u8> {
    format!("bedrock-file-v1:{vault_id}:{file_id}").into_bytes()
}

/// Creates a new vault key and wraps it with the password.
pub fn create_envelope(password: &str, params: KdfParams) -> Result<(KeyEnvelope, VaultKey), CryptoError> {
    let key = VaultKey::generate();
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let kek = derive_kek(password, &salt, params)?;
    let cipher = <XChaCha20Poly1305 as KeyInit>::new_from_slice(kek.as_slice()).expect("32-byte key");
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let wrapped = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: &key.raw, aad: ENVELOPE_AAD })
        .expect("XChaCha20-Poly1305 encryption does not fail");
    let envelope = KeyEnvelope {
        kdf: "argon2id".into(),
        salt: B64.encode(salt),
        m_cost: params.m_cost,
        t_cost: params.t_cost,
        p_cost: params.p_cost,
        nonce: B64.encode(nonce),
        wrapped_key: B64.encode(wrapped),
    };
    Ok((envelope, key))
}

/// Unwraps the vault key with the password.
pub fn open_envelope(env: &KeyEnvelope, password: &str) -> Result<VaultKey, CryptoError> {
    if env.kdf != "argon2id" {
        return Err(CryptoError::Kdf(format!("неизвестный KDF {}", env.kdf)));
    }
    // A hostile server could ask for absurd costs to hang the client.
    if env.m_cost > 1024 * 1024 || env.t_cost > 20 || env.p_cost > 16 {
        return Err(CryptoError::Kdf("слишком большие параметры".into()));
    }
    let decode = |s: &str| B64.decode(s).map_err(|_| CryptoError::Corrupt("конверт ключа не в base64"));
    let salt = decode(&env.salt)?;
    let nonce = decode(&env.nonce)?;
    let wrapped = decode(&env.wrapped_key)?;
    if nonce.len() != NONCE_LEN {
        return Err(CryptoError::Corrupt("длина nonce"));
    }
    let params = KdfParams { m_cost: env.m_cost, t_cost: env.t_cost, p_cost: env.p_cost };
    let kek = derive_kek(password, &salt, params)?;
    let cipher = <XChaCha20Poly1305 as KeyInit>::new_from_slice(kek.as_slice()).expect("32-byte key");
    let raw = Zeroizing::new(
        cipher
            .decrypt(XNonce::from_slice(&nonce), Payload { msg: &wrapped, aad: ENVELOPE_AAD })
            .map_err(|_| CryptoError::WrongPassword)?,
    );
    let raw: [u8; 32] = raw.as_slice().try_into().map_err(|_| CryptoError::Corrupt("длина ключа"))?;
    Ok(VaultKey::from_raw(raw))
}

fn derive_kek(password: &str, salt: &[u8], p: KdfParams) -> Result<Zeroizing<[u8; 32]>, CryptoError> {
    let params = Params::new(p.m_cost, p.t_cost, p.p_cost, Some(32)).map_err(|e| CryptoError::Kdf(e.to_string()))?;
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut out[..])
        .map_err(|e| CryptoError::Kdf(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let (env, key) = create_envelope("hunter22", KdfParams::INSECURE_TEST).unwrap();
        let opened = open_envelope(&env, "hunter22").unwrap();
        assert_eq!(opened.to_base64(), key.to_base64());
        assert!(matches!(open_envelope(&env, "wrong"), Err(CryptoError::WrongPassword)));
    }

    #[test]
    fn blob_roundtrip_and_tamper() {
        let key = VaultKey::generate();
        let id = key.file_id("Folder/Note.md");
        assert_eq!(id.len(), 64);
        let blob = key.seal("v1", &id, "Folder/Note.md", "привет".as_bytes());
        let (path, content) = key.open("v1", &id, &blob).unwrap();
        assert_eq!(path, "Folder/Note.md");
        assert_eq!(content, "привет".as_bytes());

        // Same blob under another vault or file id must not open.
        assert!(key.open("v2", &id, &blob).is_err());
        assert!(key.open("v1", &key.file_id("Other.md"), &blob).is_err());
        let mut bad = blob.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(key.open("v1", &id, &bad).is_err());
    }
}
