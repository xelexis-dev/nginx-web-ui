use super::{CertificateError, Result, validation::validate_domains};
use chrono::{DateTime, Utc};
use rcgen::{
    CertificateParams, CustomExtension, DistinguishedName, KeyPair, PKCS_RSA_SHA256, PublicKeyData,
    RsaKeySize,
};
use serde::{Deserialize, Serialize};
use x509_parser::prelude::*;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsrMaterial {
    pub csr_pem: String,
    pub key_pem: String,
}

/// RSA-2048 PKCS#10, empty subject, ordered DNS SANs and noncritical KU.
pub fn create_csr(domains: &[String]) -> Result<CsrMaterial> {
    validate_domains(domains, true)?;
    let key = KeyPair::generate_rsa_for(&PKCS_RSA_SHA256, RsaKeySize::_2048)?;
    let mut params = CertificateParams::new(domains.to_vec())?;
    params.distinguished_name = DistinguishedName::new();
    // rcgen's built-in KeyUsage is critical; acme-client's CSR uses a
    // noncritical digitalSignature + keyEncipherment BIT STRING (RFC 5280).
    params
        .custom_extensions
        .push(CustomExtension::from_oid_content(
            &[2, 5, 29, 15],
            vec![0x03, 0x02, 0x05, 0xa0],
        ));
    let csr = params.serialize_request(&key)?;
    Ok(CsrMaterial {
        csr_pem: csr.pem()?,
        key_pem: key.serialize_pem(),
    })
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedCertificate {
    pub domains: Vec<String>,
    pub not_after: DateTime<Utc>,
    pub not_before: DateTime<Utc>,
}

pub fn certificate_expiry(cert_pem: &str) -> Result<DateTime<Utc>> {
    let (_, pem) =
        x509_parser::pem::parse_x509_pem(cert_pem.as_bytes()).map_err(|_| invalid_cert())?;
    let (_, cert) = X509Certificate::from_der(&pem.contents).map_err(|_| invalid_cert())?;
    timestamp(cert.validity().not_after.timestamp())
}

/// Extract display names and check leaf/key correspondence. This does not
/// perform trust validation or RFC 9525 hostname verification.
pub fn parse_certificate_pem(cert_pem: &str, key_pem: &str) -> Result<ParsedCertificate> {
    let (_, pem) =
        x509_parser::pem::parse_x509_pem(cert_pem.as_bytes()).map_err(|_| invalid_cert())?;
    if pem.label != "CERTIFICATE" {
        return Err(invalid_cert());
    }
    let (_, cert) = X509Certificate::from_der(&pem.contents).map_err(|_| invalid_cert())?;
    let key = KeyPair::from_pem(key_pem).map_err(|_| {
        CertificateError::Invalid(if key_pem.contains("ENCRYPTED") {
            "암호화된 개인키는 지원하지 않습니다 — 암호를 제거한 PEM을 올려주세요.".into()
        } else {
            "개인키 PEM을 해석할 수 없습니다.".into()
        })
    })?;
    if key.subject_public_key_info() != cert.public_key().raw {
        return Err(CertificateError::Invalid(
            "개인키가 인증서와 일치하지 않습니다.".into(),
        ));
    }
    let mut domains = Vec::new();
    if let Some(san) = cert
        .subject_alternative_name()
        .map_err(|_| invalid_cert())?
    {
        for name in &san.value.general_names {
            if let GeneralName::DNSName(name) = name {
                let name = name.to_lowercase();
                if !name.is_empty() && !domains.contains(&name) {
                    domains.push(name);
                }
            }
        }
    }
    if domains.is_empty() {
        if let Some(cn) = cert
            .subject()
            .iter_common_name()
            .find_map(|cn| cn.as_str().ok())
        {
            if !cn.is_empty() {
                domains.push(cn.to_lowercase());
            }
        }
    }
    if domains.is_empty() {
        return Err(CertificateError::Invalid(
            "인증서에서 도메인(SAN/CN)을 찾을 수 없습니다.".into(),
        ));
    }
    Ok(ParsedCertificate {
        domains,
        not_after: timestamp(cert.validity().not_after.timestamp())?,
        not_before: timestamp(cert.validity().not_before.timestamp())?,
    })
}

pub fn csr_der(pem: &str) -> Result<Vec<u8>> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|_| CertificateError::Invalid("CSR PEM을 해석할 수 없습니다.".into()))?;
    if pem.label != "CERTIFICATE REQUEST" && pem.label != "NEW CERTIFICATE REQUEST" {
        return Err(CertificateError::Invalid(
            "CSR PEM을 해석할 수 없습니다.".into(),
        ));
    }
    let (_, csr) =
        x509_parser::certification_request::X509CertificationRequest::from_der(&pem.contents)
            .map_err(|_| CertificateError::Invalid("CSR을 해석할 수 없습니다.".into()))?;
    csr.verify_signature()
        .map_err(|_| CertificateError::Invalid("CSR 서명이 유효하지 않습니다.".into()))?;
    Ok(pem.contents)
}
fn invalid_cert() -> CertificateError {
    CertificateError::Invalid("인증서 PEM을 해석할 수 없습니다.".into())
}
fn timestamp(seconds: i64) -> Result<DateTime<Utc>> {
    DateTime::from_timestamp(seconds, 0)
        .ok_or_else(|| CertificateError::Invalid("인증서 유효기간을 해석할 수 없습니다.".into()))
}
