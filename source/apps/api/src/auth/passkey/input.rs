use super::PasskeyError;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;

const MAX_JSON: usize = 64 * 1024;
pub(super) const MAX_AUTH_DATA: usize = 16 * 1024;
const MAX_CLIENT_DATA: usize = 4096;
const MAX_KEY: usize = 2048;

// Bound the whole document before serde allocates, then every consumed field before decoding.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credential<'a> {
    #[serde(borrow)]
    id: &'a str,
    #[serde(borrow)]
    raw_id: &'a str,
    #[serde(rename = "type", borrow)]
    kind: &'a str,
    #[serde(borrow)]
    response: Response<'a>,
    #[serde(default, borrow, deserialize_with = "present")]
    authenticator_attachment: Option<&'a str>,
    #[serde(default)]
    client_extension_results: Extensions,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response<'a> {
    #[serde(rename = "clientDataJSON", borrow)]
    client_data_json: &'a str,
    #[serde(borrow)]
    authenticator_data: &'a str,
    #[serde(default, borrow, deserialize_with = "present")]
    public_key: Option<&'a str>,
    #[serde(default, deserialize_with = "present")]
    public_key_algorithm: Option<i64>,
    #[serde(default, borrow, deserialize_with = "present")]
    signature: Option<&'a str>,
    #[serde(default, borrow, deserialize_with = "present")]
    user_handle: Option<&'a str>,
    #[serde(default, borrow)]
    transports: Vec<&'a str>,
    #[serde(default, borrow, deserialize_with = "present")]
    attestation_object: Option<&'a str>,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Extensions {
    #[serde(default, deserialize_with = "present")]
    cred_props: Option<CredProps>,
}
#[derive(Deserialize)]
struct CredProps {
    #[serde(default, deserialize_with = "present")]
    rk: Option<bool>,
}

// serde's Option normally treats present null as absent; browser JSON members are non-nullable.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

pub struct RegistrationInput {
    pub(super) raw_id: Vec<u8>,
    pub(super) client_data: Vec<u8>,
    pub(super) auth_data: Vec<u8>,
    pub(super) public_key: Vec<u8>,
    pub(super) algorithm: i64,
    pub(super) resident: Option<bool>,
    pub(super) transports: Vec<String>,
}
pub struct AssertionInput {
    pub(super) raw_id: Vec<u8>,
    pub(super) client_data: Vec<u8>,
    pub(super) auth_data: Vec<u8>,
    pub(super) signature: Vec<u8>,
    pub(super) user_handle: Option<Vec<u8>>,
}
impl AssertionInput {
    pub fn raw_id(&self) -> &[u8] {
        &self.raw_id
    }
    pub fn user_handle(&self) -> Option<&[u8]> {
        self.user_handle.as_deref()
    }
}

pub(super) fn decode(value: &str, limit: usize) -> Result<Vec<u8>, PasskeyError> {
    if value.len() > limit.div_ceil(3) * 4 {
        return Err(PasskeyError::FieldTooLarge);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| PasskeyError::InvalidEncoding)?;
    if bytes.len() > limit {
        return Err(PasskeyError::FieldTooLarge);
    }
    Ok(bytes)
}
fn parse(json: &str) -> Result<(Credential<'_>, Vec<u8>), PasskeyError> {
    if json.len() > MAX_JSON {
        return Err(PasskeyError::FieldTooLarge);
    }
    let object: serde_json::Value =
        serde_json::from_str(json).map_err(|_| PasskeyError::InvalidInput)?;
    // Derived serde structs also accept sequences; these WebAuthn members must be objects.
    if !object.is_object()
        || !object["response"].is_object()
        || object
            .get("clientExtensionResults")
            .is_some_and(|extensions| {
                !extensions.is_object()
                    || extensions
                        .get("credProps")
                        .is_some_and(|props| !props.is_object())
            })
    {
        return Err(PasskeyError::InvalidInput);
    }
    let credential: Credential<'_> =
        serde_json::from_str(json).map_err(|_| PasskeyError::InvalidInput)?;
    if credential.kind != "public-key" {
        return Err(PasskeyError::CredentialType);
    }
    if credential.id.len() > 1364
        || credential
            .authenticator_attachment
            .is_some_and(|v| v.len() > 32)
    {
        return Err(PasskeyError::FieldTooLarge);
    }
    let raw_id = decode(credential.raw_id, 1023)?;
    if raw_id.is_empty() {
        return Err(PasskeyError::CredentialId);
    }
    if credential.id != URL_SAFE_NO_PAD.encode(&raw_id) {
        return Err(PasskeyError::CredentialIdMismatch);
    }
    let r = &credential.response;
    if r.transports.len() > 8 || r.transports.iter().any(|v| v.len() > 32) {
        return Err(PasskeyError::FieldTooLarge);
    }
    // Bound optional fields even when they belong to the other ceremony.
    for (value, limit) in [
        (r.public_key, MAX_KEY),
        (r.signature, 1024),
        (r.user_handle, 64),
        (r.attestation_object, MAX_AUTH_DATA + 1024),
    ] {
        if let Some(value) = value {
            decode(value, limit)?;
        }
    }
    Ok((credential, raw_id))
}
pub fn parse_registration(json: &str) -> Result<RegistrationInput, PasskeyError> {
    let (credential, raw_id) = parse(json)?;
    let r = credential.response;
    Ok(RegistrationInput {
        raw_id,
        client_data: decode(r.client_data_json, MAX_CLIENT_DATA)?,
        auth_data: decode(r.authenticator_data, MAX_AUTH_DATA)?,
        public_key: decode(r.public_key.ok_or(PasskeyError::MissingPublicKey)?, MAX_KEY)?,
        algorithm: r
            .public_key_algorithm
            .ok_or(PasskeyError::UnsupportedAlgorithm)?,
        resident: credential
            .client_extension_results
            .cred_props
            .and_then(|p| p.rk),
        transports: r.transports.into_iter().map(str::to_owned).collect(),
    })
}
pub fn parse_assertion(json: &str) -> Result<AssertionInput, PasskeyError> {
    let (credential, raw_id) = parse(json)?;
    let r = credential.response;
    Ok(AssertionInput {
        raw_id,
        client_data: decode(r.client_data_json, MAX_CLIENT_DATA)?,
        auth_data: decode(r.authenticator_data, MAX_AUTH_DATA)?,
        signature: decode(r.signature.ok_or(PasskeyError::Signature)?, 1024)?,
        user_handle: r.user_handle.map(|v| decode(v, 64)).transpose()?,
    })
}
