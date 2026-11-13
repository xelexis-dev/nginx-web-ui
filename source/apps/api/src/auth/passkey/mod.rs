//! Pure WebAuthn validation; persistence and ceremony state belong to the caller.
mod input;
mod verify;
pub use input::{AssertionInput, RegistrationInput, parse_assertion, parse_registration};
pub use verify::{availability, counter_accepts, verify_assertion, verify_registration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasskeyAlg {
    Es256,
    Rs256,
}
pub struct Expected<'a> {
    pub rp_id: &'a str,
    pub origin: &'a str,
    pub challenge: &'a [u8],
}
#[derive(Debug)]
pub struct VerifiedRegistration {
    pub credential_id: Vec<u8>,
    pub public_key_spki: Vec<u8>,
    pub algorithm: PasskeyAlg,
    pub sign_count: u32,
    pub backup_eligible: bool,
    pub backup_state: bool,
    pub transports: Vec<String>,
}
pub struct StoredCredential<'a> {
    pub public_key_spki: &'a [u8],
    pub algorithm: PasskeyAlg,
    pub backup_eligible: bool,
    pub rp_id: &'a str,
    pub origin: &'a str,
    pub user_handle: &'a [u8],
}
#[derive(Debug)]
pub struct VerifiedAssertion {
    pub sign_count: u32,
    pub backup_state: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Availability {
    Available { rp_id: String, origin: String },
    Unavailable(UnavailableReason),
}
#[derive(Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    InsecureOrigin,
    IpHost,
    AuthUrlHttp,
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PasskeyError {
    #[error("패스키 정보를 확인할 수 없습니다.")]
    InvalidInput,
    #[error("패스키 정보가 너무 큽니다.")]
    FieldTooLarge,
    #[error("패스키 정보의 인코딩이 올바르지 않습니다.")]
    InvalidEncoding,
    #[error("패스키 형식이 올바르지 않습니다.")]
    CredentialType,
    #[error("패스키 식별자가 올바르지 않습니다.")]
    CredentialId,
    #[error("패스키 식별자가 일치하지 않습니다.")]
    CredentialIdMismatch,
    #[error("브라우저의 패스키 정보를 읽을 수 없습니다.")]
    ClientData,
    #[error("패스키 요청 종류가 일치하지 않습니다.")]
    ClientType,
    #[error("패스키 확인 요청이 일치하지 않습니다. 다시 시도해 주세요.")]
    Challenge,
    #[error("패스키를 요청한 주소가 일치하지 않습니다.")]
    Origin,
    #[error("다른 사이트 안에서는 패스키를 사용할 수 없습니다.")]
    CrossOrigin,
    #[error("프레임 안에서는 패스키를 사용할 수 없습니다.")]
    TopOrigin,
    #[error("인증기의 정보를 읽을 수 없습니다.")]
    AuthData,
    #[error("패스키의 도메인이 일치하지 않습니다.")]
    RpIdHash,
    #[error("인증기에서 사용자 동작을 확인하지 못했습니다.")]
    UserPresence,
    #[error("인증기에서 본인 확인을 완료해 주세요.")]
    UserVerification,
    #[error("패스키 등록 정보가 없습니다.")]
    AttestedData,
    #[error("패스키 백업 정보가 올바르지 않습니다.")]
    BackupFlags,
    #[error("패스키의 백업 가능 여부가 변경되었습니다.")]
    BackupEligibility,
    #[error("지원하지 않는 패스키 알고리즘입니다.")]
    UnsupportedAlgorithm,
    #[error(
        "브라우저가 패스키 공개키를 제공하지 않았습니다. 다른 브라우저나 인증기를 사용해 주세요."
    )]
    MissingPublicKey,
    #[error("패스키 공개키가 올바르지 않습니다.")]
    InvalidPublicKey,
    #[error("패스키는 P-256 키를 사용해야 합니다.")]
    InvalidCurve,
    #[error("RSA 패스키는 2048비트 이상의 키를 사용해야 합니다.")]
    WeakRsaKey,
    #[error("로그인할 수 있는 검색 가능한 패스키가 필요합니다.")]
    NotDiscoverable,
    #[error("패스키의 사용자 정보가 일치하지 않습니다.")]
    UserHandle,
    #[error("이 도메인에서 등록한 패스키가 아닙니다.")]
    StoredRpId,
    #[error("이 주소에서 등록한 패스키가 아닙니다.")]
    StoredOrigin,
    #[error("패스키 서명을 확인하지 못했습니다.")]
    Signature,
}
#[cfg(test)]
mod tests;
