//! At-rest sealing, byte-compatible with the TypeScript `@repo/crypto` `enc:v1:` format.
//!
//! Format (not an RFC): `enc:v1:` + base64 (RFC 4648 §4, padded) of `iv(12) ‖ tag(16) ‖ ciphertext`,
//! AES-256-GCM per NIST SP 800-38D with a 96-bit random IV and a 128-bit tag, no associated data.
//! The 32-byte key lives as 64 hex characters in the data-key file (`DATA_KEY_FILE`).
//!
//! Compatibility contract: every value the TS code sealed must open here and vice versa
//! (golden vectors in `auth::compat_tests`). Losing or overwriting the key file makes every
//! sealed value unreadable — TLS private keys, the ACME account key, TOTP secrets, node,
//! DNS, SMTP, OAuth and webhook secrets — so an existing but invalid key file is an error
//! and is never overwritten.

use std::fmt;
use std::fs;
use std::io;
#[cfg(test)]
use std::io::Write as _;
use std::path::Path;

use aes_gcm::aead::{Nonce, Tag};
use aes_gcm::{AeadInOut, Aes256Gcm, KeyInit};
use base64::Engine as _;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::{DecodePaddingMode, general_purpose};

pub const SEAL_PREFIX: &str = "enc:v1:";
const IV_BYTES: usize = 12;
const TAG_BYTES: usize = 16;

/// Standard alphabet, padded on output (what Node's `toString("base64")` writes), padding
/// optional on input.
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// ECMAScript `WhiteSpace ∪ LineTerminator` — the set `String.prototype.trim` and regex `\s`
/// use. It differs from Rust's `char::is_whitespace` by U+FEFF (JS only) and U+0085 (Rust only).
pub(crate) fn is_ecmascript_whitespace(c: char) -> bool {
    c == '\u{feff}' || (c.is_whitespace() && c != '\u{85}')
}

/// `String.prototype.trim`.
pub(crate) fn js_trim(value: &str) -> &str {
    value.trim_matches(is_ecmascript_whitespace)
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("data key file {path} exists but does not contain a 32-byte hex key")]
    InvalidKeyFile { path: String },
    #[error("data key file I/O failed for {path}: {source}")]
    Io { path: String, source: io::Error },
    #[error("the system random number generator failed")]
    Random,
    #[error("sealing failed")]
    Seal,
}

/// Mirrors the TS `SealedState` classification used by the secrets status screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealedState {
    /// Sealed and opens with the current key.
    SealedOk,
    /// Sealed but does not open — usually a lost or replaced data key. The only incident state.
    SealedBroken,
    /// Legacy plaintext written before sealing existed. Works; not yet sealed.
    Plaintext,
    /// No value stored.
    Absent,
}

/// Holds the data key. The key never leaves this type and is not printed by `Debug`.
pub struct SecretBox {
    key: [u8; 32],
}

impl fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretBox(<redacted>)")
    }
}

impl SecretBox {
    pub fn from_key(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// Migration must never replace a lost key with a newly generated one.
    pub fn load_existing(path: &Path) -> Result<Self, CryptoError> {
        use std::io::Read as _;
        let io_err = |source| CryptoError::Io { path: path.display().to_string(), source };
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let file = options.open(path).map_err(io_err)?;
        if !file.metadata().map_err(io_err)?.is_file() {
            return Err(CryptoError::InvalidKeyFile { path: path.display().to_string() });
        }
        let mut content = String::new();
        file.take(4097).read_to_string(&mut content).map_err(io_err)?;
        if content.len() > 4096 {
            return Err(CryptoError::InvalidKeyFile { path: path.display().to_string() });
        }
        parse_key_file(path, &content)
    }

    /// TS `loadKey`: read the key file; if it does not exist, create it (directory 0700, file
    /// 0600, exclusive create so a concurrent creator can never be overwritten) and re-read on a
    /// lost race. Complete bytes are fsynced in a private temporary file before hard-link publication.
    /// An existing file that is not a 64-hex key is an error, never replaced.
    #[cfg(test)]
    pub fn load_or_create(path: &Path) -> Result<Self, CryptoError> {
        let io_err = |source| CryptoError::Io { path: path.display().to_string(), source };
        match read_key_file(path) {
            Ok(result) => return result,
            Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(io_err(err)),
            Err(_) => {}
        }

        let mut key = [0u8; 32];
        getrandom::fill(&mut key).map_err(|_| CryptoError::Random)?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            builder.create(dir).map_err(io_err)?;
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let parent = path.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| CryptoError::Random)?;
        let temporary = parent.join(format!(".nwui-key-{}.tmp", hex::encode(nonce)));
        let mut file = options.open(&temporary).map_err(io_err)?;
        let result = (|| {
            file.write_all(hex::encode(key).as_bytes()).map_err(io_err)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o600)).map_err(io_err)?;
            }
            file.sync_all().map_err(io_err)?;
            // Publish only complete bytes; another creator's key must never be overwritten.
            let secret = match fs::hard_link(&temporary, path) {
                Ok(()) => Self::from_key(key),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => read_key_file(path).map_err(io_err)??,
                Err(error) => return Err(io_err(error)),
            };
            #[cfg(unix)]
            fs::File::open(parent).and_then(|directory| directory.sync_all()).map_err(io_err)?;
            Ok(secret)
        })();
        drop(file);
        let _ = fs::remove_file(&temporary);
        result
    }

    pub fn seal(&self, plain: &str) -> Result<String, CryptoError> {
        let mut iv = [0u8; IV_BYTES];
        getrandom::fill(&mut iv).map_err(|_| CryptoError::Random)?;
        let mut body = plain.as_bytes().to_vec();
        let tag = self
            .cipher()
            .encrypt_inout_detached(&Nonce::<Aes256Gcm>::from(iv), b"", body.as_mut_slice().into())
            .map_err(|_| CryptoError::Seal)?;
        let mut raw = Vec::with_capacity(IV_BYTES + TAG_BYTES + body.len());
        raw.extend_from_slice(&iv);
        raw.extend_from_slice(&tag);
        raw.extend_from_slice(&body);
        Ok(format!("{SEAL_PREFIX}{}", general_purpose::STANDARD.encode(raw)))
    }

    /// For columns that were always sealed: `None` when absent, not sealed, or not openable.
    pub fn open(&self, stored: Option<&str>) -> Option<String> {
        let stored = stored.filter(|s| !s.is_empty())?;
        if !is_sealed(stored) {
            return None;
        }
        self.decrypt(stored)
    }

    /// For columns that predate sealing: legacy plaintext is returned unchanged.
    pub fn open_maybe_plain(&self, stored: Option<&str>) -> Option<String> {
        let stored = stored.filter(|s| !s.is_empty())?;
        if is_sealed(stored) { self.decrypt(stored) } else { Some(stored.to_owned()) }
    }

    /// `classifySealed(stored, open(stored) !== null)` as the secrets status page computes it.
    pub fn sealed_state(&self, stored: Option<&str>) -> SealedState {
        let Some(value) = stored.map(js_trim).filter(|v| !v.is_empty()) else {
            return SealedState::Absent;
        };
        if !is_sealed(value) {
            return SealedState::Plaintext;
        }
        if self.open(stored).is_some() { SealedState::SealedOk } else { SealedState::SealedBroken }
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(&self.key.into())
    }

    fn decrypt(&self, stored: &str) -> Option<String> {
        let raw = BASE64.decode(&stored[SEAL_PREFIX.len()..]).ok()?;
        // Node would also accept a shorter tag here; we require the 16 bytes seal always writes.
        if raw.len() < IV_BYTES + TAG_BYTES {
            return None;
        }
        let (iv, rest) = raw.split_at(IV_BYTES);
        let (tag, body) = rest.split_at(TAG_BYTES);
        let nonce = Nonce::<Aes256Gcm>::try_from(iv).ok()?;
        let tag = Tag::<Aes256Gcm>::try_from(tag).ok()?;
        let mut body = body.to_vec();
        self.cipher().decrypt_inout_detached(&nonce, b"", body.as_mut_slice().into(), &tag).ok()?;
        // Node's `toString("utf8")` is lossy; sealed JS strings are valid UTF-8 anyway.
        Some(String::from_utf8_lossy(&body).into_owned())
    }
}

/// `Ok(Ok(key))` valid, `Ok(Err(InvalidKeyFile))` present but invalid, `Err(io)` unreadable.
#[cfg(test)]
fn read_key_file(path: &Path) -> io::Result<Result<SecretBox, CryptoError>> {
    let content = fs::read_to_string(path)?;
    Ok(parse_key_file(path, &content))
}

pub(crate) fn parse_key_file(path: &Path, content: &str) -> Result<SecretBox, CryptoError> {
    let hex_key = js_trim(content);
    let mut key = [0u8; 32];
    if hex_key.len() == 64 && hex::decode_to_slice(hex_key, &mut key).is_ok() {
        Ok(SecretBox::from_key(key))
    } else {
        Err(CryptoError::InvalidKeyFile { path: path.display().to_string() })
    }
}

pub fn is_sealed(stored: &str) -> bool {
    stored.starts_with(SEAL_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    const KEY: [u8; 32] = [7u8; 32];

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nwui-crypto-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn seal_then_open_round_trips_utf8_and_empty() {
        let secret_box = SecretBox::from_key(KEY);
        for plain in ["", "ascii", "한글 🔐", &"L".repeat(70_000)] {
            let sealed = secret_box.seal(plain).unwrap();
            assert!(sealed.starts_with(SEAL_PREFIX));
            assert_eq!(secret_box.open(Some(&sealed)).as_deref(), Some(plain));
            assert_eq!(secret_box.open_maybe_plain(Some(&sealed)).as_deref(), Some(plain));
        }
    }

    #[test]
    fn seal_uses_a_fresh_iv_each_time() {
        let secret_box = SecretBox::from_key(KEY);
        assert_ne!(secret_box.seal("same").unwrap(), secret_box.seal("same").unwrap());
    }

    #[test]
    fn open_rejects_absent_plaintext_and_tampering() {
        let secret_box = SecretBox::from_key(KEY);
        assert_eq!(secret_box.open(None), None);
        assert_eq!(secret_box.open(Some("")), None);
        // `open` is for columns that were always sealed: plaintext is refused.
        assert_eq!(secret_box.open(Some("-----BEGIN PRIVATE KEY-----")), None);
        // Non-ASCII plaintext is refused too, without slicing through a UTF-8 character.
        assert_eq!(secret_box.open(Some("비밀번호 값")), None);
        assert_eq!(secret_box.open_maybe_plain(Some("비밀번호 값")).as_deref(), Some("비밀번호 값"));

        let sealed = secret_box.seal("payload").unwrap();
        let body = &sealed[SEAL_PREFIX.len()..];
        let mut raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body).unwrap();
        for index in [0, 12, raw.len() - 1] {
            raw[index] ^= 0x01; // IV, tag and ciphertext each authenticated
            let tampered = format!("{SEAL_PREFIX}{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &raw));
            assert_eq!(secret_box.open(Some(&tampered)), None, "flip at {index}");
            raw[index] ^= 0x01;
        }
    }

    #[test]
    fn open_rejects_short_and_garbage_input_without_panicking() {
        let secret_box = SecretBox::from_key(KEY);
        let short = format!("{SEAL_PREFIX}{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [1u8; 27]));
        for stored in [short.as_str(), "enc:v1:", "enc:v1:!!!!", "enc:v1:AAAA"] {
            assert_eq!(secret_box.open(Some(stored)), None, "{stored}");
        }
    }

    #[test]
    fn a_different_key_cannot_open() {
        let sealed = SecretBox::from_key(KEY).seal("payload").unwrap();
        assert_eq!(SecretBox::from_key([8u8; 32]).open(Some(&sealed)), None);
    }

    #[test]
    fn open_maybe_plain_passes_legacy_plaintext_through() {
        let secret_box = SecretBox::from_key(KEY);
        assert_eq!(secret_box.open_maybe_plain(None), None);
        assert_eq!(secret_box.open_maybe_plain(Some("")), None);
        assert_eq!(secret_box.open_maybe_plain(Some("JBSWY3DPEHPK3PXP")).as_deref(), Some("JBSWY3DPEHPK3PXP"));
    }

    #[test]
    fn sealed_state_classifies_like_the_ts_helper() {
        let secret_box = SecretBox::from_key(KEY);
        let sealed = secret_box.seal("x").unwrap();
        assert_eq!(secret_box.sealed_state(None), SealedState::Absent);
        assert_eq!(secret_box.sealed_state(Some("")), SealedState::Absent);
        // TS `classifySealed` trims first; a future `enc:v2:` is not mistaken for a broken v1.
        assert_eq!(secret_box.sealed_state(Some(" \u{feff}\n")), SealedState::Absent);
        assert_eq!(secret_box.sealed_state(Some("enc:v2:abc")), SealedState::Plaintext);
        assert_eq!(secret_box.sealed_state(Some("plain")), SealedState::Plaintext);
        assert_eq!(secret_box.sealed_state(Some(&sealed)), SealedState::SealedOk);
        assert_eq!(SecretBox::from_key([9u8; 32]).sealed_state(Some(&sealed)), SealedState::SealedBroken);
    }

    #[test]
    fn debug_does_not_print_the_key() {
        let printed = format!("{:?}", SecretBox::from_key(KEY));
        assert!(!printed.contains('7'), "{printed}");
    }

    #[test]
    fn load_or_create_creates_a_private_hex_key_once() {
        let dir = scratch_dir("create");
        let file = dir.join("nested").join("data_key");
        let first = SecretBox::load_or_create(&file).unwrap();
        let content = fs::read_to_string(&file).unwrap();
        assert_eq!(content.len(), 64);
        assert!(content.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // Second load reads the same key: values sealed before keep opening.
        let sealed = first.seal("persisted").unwrap();
        let second = SecretBox::load_or_create(&file).unwrap();
        assert_eq!(second.open(Some(&sealed)).as_deref(), Some("persisted"));
        assert_eq!(fs::read_to_string(&file).unwrap(), content);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_or_create_accepts_uppercase_hex_with_trailing_newline() {
        let dir = scratch_dir("upper");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data_key");
        fs::write(&file, format!("{}\n", "0A".repeat(32))).unwrap();
        let loaded = SecretBox::load_or_create(&file).unwrap();
        let sealed = SecretBox::from_key([0x0a; 32]).seal("x").unwrap();
        assert_eq!(loaded.open(Some(&sealed)).as_deref(), Some("x"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_or_create_never_overwrites_an_invalid_existing_file() {
        let dir = scratch_dir("invalid");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data_key");
        for bad in ["abc", &"0".repeat(63), &"0".repeat(65), &format!("{}g", "0".repeat(63)), ""] {
            fs::write(&file, bad).unwrap();
            let err = SecretBox::load_or_create(&file).unwrap_err();
            assert!(matches!(err, CryptoError::InvalidKeyFile { .. }), "{bad:?}: {err}");
            assert_eq!(fs::read_to_string(&file).unwrap(), bad, "must not overwrite");
        }
        let _ = fs::remove_dir_all(dir);
    }
    #[cfg(unix)]
    #[test]
    fn audit_i9_interrupted_key_publication_keeps_final_path_absent() {
        const CHILD_PATH: &str = "NWUI_KEY_CRASH_TEST_PATH";
        if let Some(path) = std::env::var_os(CHILD_PATH) {
            let _ = SecretBox::load_or_create(Path::new(&path));
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("data_key");
        let status = std::process::Command::new("sh")
            .args(["-c", "ulimit -f 0; exec \"$1\" --exact crypto::tests::audit_i9_interrupted_key_publication_keeps_final_path_absent --nocapture", "sh"])
            .arg(std::env::current_exe().unwrap())
            .env(CHILD_PATH, &path)
            .status().unwrap();
        assert_eq!(std::os::unix::process::ExitStatusExt::signal(&status), Some(25), "the child must hit SIGXFSZ during its first key-file write");
        assert!(!path.exists(), "an interrupted write published an empty final data key");
        SecretBox::load_or_create(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap().len(), 64);
    }

    #[test]
    fn audit_i9_concurrent_creators_all_load_the_published_key() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("data_key");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles = (0..8).map(|_| {
            let (path, barrier) = (path.clone(), barrier.clone());
            std::thread::spawn(move || { barrier.wait(); SecretBox::load_or_create(&path).unwrap() })
        }).collect::<Vec<_>>();
        let keys = handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>();
        let sealed = keys[0].seal("shared key").unwrap();
        assert!(keys.iter().all(|key| key.open(Some(&sealed)).as_deref() == Some("shared key")));
        assert_eq!(std::fs::read(&path).unwrap().len(), 64);
    }

}
