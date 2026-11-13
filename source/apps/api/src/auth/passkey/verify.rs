use super::*;
use aws_lc_rs::signature::{self, ParsedPublicKey, UnparsedPublicKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use x509_parser::{prelude::FromDer, public_key::PublicKey, x509::SubjectPublicKeyInfo};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientData<'a> {
    #[serde(rename = "type", borrow)]
    kind: &'a str,
    #[serde(borrow)]
    challenge: &'a str,
    #[serde(borrow)]
    origin: &'a str,
    #[serde(default)]
    cross_origin: Option<serde_json::Value>,
    #[serde(default)]
    top_origin: Option<serde_json::Value>,
}
fn client_data(bytes: &[u8], expected: &Expected<'_>, kind: &str) -> Result<(), PasskeyError> {
    let c: ClientData<'_> = serde_json::from_slice(bytes).map_err(|_| PasskeyError::ClientData)?;
    if c.kind != kind {
        return Err(PasskeyError::ClientType);
    }
    let challenge = input::decode(c.challenge, 1024)?;
    if !bool::from(challenge.ct_eq(expected.challenge)) {
        return Err(PasskeyError::Challenge);
    }
    if c.origin != expected.origin {
        return Err(PasskeyError::Origin);
    }
    // Presence matters: null is not an omitted optional Boolean or an absent topOrigin.
    let object: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| PasskeyError::ClientData)?;
    if object.get("crossOrigin").is_some() && c.cross_origin != Some(serde_json::Value::Bool(false))
    {
        return Err(PasskeyError::CrossOrigin);
    }
    if object.get("topOrigin").is_some() || c.top_origin.is_some() {
        return Err(PasskeyError::TopOrigin);
    }
    Ok(())
}
struct AuthData {
    flags: u8,
    count: u32,
}
impl AuthData {
    fn be(&self) -> bool {
        self.flags & 0x08 != 0
    }
    fn bs(&self) -> bool {
        self.flags & 0x10 != 0
    }
}
fn auth_data(bytes: &[u8], expected: &Expected<'_>) -> Result<AuthData, PasskeyError> {
    if bytes.len() < 37 {
        return Err(PasskeyError::AuthData);
    }
    if !bool::from(bytes[..32].ct_eq(&Sha256::digest(expected.rp_id.as_bytes())[..])) {
        return Err(PasskeyError::RpIdHash);
    }
    let flags = bytes[32];
    if flags & 1 == 0 {
        return Err(PasskeyError::UserPresence);
    }
    if flags & 4 == 0 {
        return Err(PasskeyError::UserVerification);
    }
    if flags & 0x18 == 0x10 {
        return Err(PasskeyError::BackupFlags);
    }
    Ok(AuthData {
        flags,
        count: u32::from_be_bytes(
            bytes[33..37]
                .try_into()
                .map_err(|_| PasskeyError::AuthData)?,
        ),
    })
}
fn public_key(bytes: &[u8], algorithm: PasskeyAlg) -> Result<ParsedPublicKey, PasskeyError> {
    let (remaining, spki) =
        SubjectPublicKeyInfo::from_der(bytes).map_err(|_| PasskeyError::InvalidPublicKey)?;
    if !remaining.is_empty() || spki.subject_public_key.unused_bits != 0 {
        return Err(PasskeyError::InvalidPublicKey);
    }
    let parsed = spki.parsed().map_err(|_| PasskeyError::InvalidPublicKey)?;
    let alg: &'static dyn signature::VerificationAlgorithm = match (algorithm, parsed) {
        (PasskeyAlg::Es256, PublicKey::EC(_)) => {
            let curve = spki
                .algorithm
                .parameters
                .as_ref()
                .and_then(|p| p.as_oid().ok());
            if curve.as_ref().map(ToString::to_string).as_deref() != Some("1.2.840.10045.3.1.7") {
                return Err(PasskeyError::InvalidCurve);
            }
            &signature::ECDSA_P256_SHA256_ASN1
        }
        (PasskeyAlg::Rs256, PublicKey::RSA(key)) => {
            let n = key
                .modulus
                .iter()
                .position(|b| *b != 0)
                .map(|i| &key.modulus[i..])
                .unwrap_or(&[]);
            let bits = n
                .first()
                .map_or(0, |b| n.len() * 8 - b.leading_zeros() as usize);
            if bits < 2048 {
                return Err(PasskeyError::WeakRsaKey);
            }
            &signature::RSA_PKCS1_2048_8192_SHA256
        }
        _ => return Err(PasskeyError::InvalidPublicKey),
    };
    UnparsedPublicKey::new(alg, bytes)
        .parse()
        .map_err(|_| PasskeyError::InvalidPublicKey)
}
pub fn verify_registration(
    input: &RegistrationInput,
    expected: &Expected<'_>,
) -> Result<VerifiedRegistration, PasskeyError> {
    client_data(&input.client_data, expected, "webauthn.create")?;
    let data = auth_data(&input.auth_data, expected)?;
    if data.flags & 0x40 == 0 {
        return Err(PasskeyError::AttestedData);
    }
    // Skip the 16-byte AAGUID; the following CBOR key is required but not decoded.
    let length = input.auth_data.get(53..55).ok_or(PasskeyError::AuthData)?;
    let length = u16::from_be_bytes([length[0], length[1]]) as usize;
    if length == 0 || length > 1023 {
        return Err(PasskeyError::CredentialId);
    }
    let id = input
        .auth_data
        .get(55..55 + length)
        .ok_or(PasskeyError::AuthData)?;
    if input.auth_data.len() == 55 + length {
        return Err(PasskeyError::AuthData);
    }
    if id != input.raw_id {
        return Err(PasskeyError::CredentialIdMismatch);
    }
    let algorithm = match input.algorithm {
        -7 => PasskeyAlg::Es256,
        -257 => PasskeyAlg::Rs256,
        _ => return Err(PasskeyError::UnsupportedAlgorithm),
    };
    public_key(&input.public_key, algorithm)?;
    if input.resident == Some(false) {
        return Err(PasskeyError::NotDiscoverable);
    }
    Ok(VerifiedRegistration {
        credential_id: input.raw_id.clone(),
        public_key_spki: input.public_key.clone(),
        algorithm,
        sign_count: data.count,
        backup_eligible: data.be(),
        backup_state: data.bs(),
        transports: input.transports.clone(),
    })
}
pub fn verify_assertion(
    input: &AssertionInput,
    expected: &Expected<'_>,
    stored: &StoredCredential<'_>,
) -> Result<VerifiedAssertion, PasskeyError> {
    client_data(&input.client_data, expected, "webauthn.get")?;
    if stored.rp_id != expected.rp_id {
        return Err(PasskeyError::StoredRpId);
    }
    if stored.origin != expected.origin {
        return Err(PasskeyError::StoredOrigin);
    }
    let data = auth_data(&input.auth_data, expected)?;
    if data.flags & 0xc0 != 0 || input.auth_data.len() != 37 {
        return Err(PasskeyError::AuthData);
    }
    if data.be() != stored.backup_eligible {
        return Err(PasskeyError::BackupEligibility);
    }
    if input.user_handle.as_deref() != Some(stored.user_handle) || stored.user_handle.is_empty() {
        return Err(PasskeyError::UserHandle);
    }
    let mut signed = input.auth_data.clone();
    signed.extend_from_slice(&Sha256::digest(&input.client_data));
    public_key(stored.public_key_spki, stored.algorithm)?
        .verify_sig(&signed, &input.signature)
        .map_err(|_| PasskeyError::Signature)?;
    Ok(VerifiedAssertion {
        sign_count: data.count,
        backup_state: data.bs(),
    })
}
/// Call while holding the credential row lock, immediately before writing the received count.
pub fn counter_accepts(stored: u32, received: u32) -> bool {
    (stored == 0 && received == 0) || received > stored
}

pub fn availability(
    ctx: &crate::request_context::RequestContext,
    auth_url: Option<&str>,
) -> Availability {
    use Availability::*;
    use UnavailableReason::*;
    let configured = auth_url.is_some();
    let Ok(url) = url::Url::parse(auth_url.unwrap_or(&ctx.external_origin)) else {
        return Unavailable(InsecureOrigin);
    };
    let host = match url.host() {
        Some(url::Host::Domain(host)) if !host.ends_with('.') => host,
        Some(url::Host::Ipv4(_) | url::Host::Ipv6(_)) => return Unavailable(IpHost),
        _ => return Unavailable(InsecureOrigin),
    };
    if url.scheme() == "http" && host != "localhost" {
        return Unavailable(if configured {
            AuthUrlHttp
        } else {
            InsecureOrigin
        });
    }
    if !matches!(url.scheme(), "http" | "https")
        || (!configured && !ctx.secure && !(url.scheme() == "http" && host == "localhost"))
    {
        return Unavailable(InsecureOrigin);
    }
    Available {
        rp_id: host.to_owned(),
        origin: url.origin().ascii_serialization(),
    }
}
