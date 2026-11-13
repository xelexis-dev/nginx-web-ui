use super::{CertificateError, Result};

pub fn validate_domain(domain: &str, allow_wildcard: bool) -> Result<()> {
    let name = if allow_wildcard {
        domain.strip_prefix("*.").unwrap_or(domain)
    } else {
        domain
    };
    if name.contains('*') {
        return Err(CertificateError::Invalid(if allow_wildcard {
            format!("와일드카드는 맨 앞 한 단계(*.example.com)만 쓸 수 있습니다: {domain}")
        } else {
            format!(
                "와일드카드 도메인은 HTTP-01로 발급할 수 없습니다. 검증 방식을 DNS-01로 바꾸세요: {domain}"
            )
        }));
    }
    let labels: Vec<_> = name.split('.').collect();
    let valid = domain.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        && labels
            .last()
            .is_some_and(|tld| tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_lowercase()));
    if !valid {
        return Err(CertificateError::Invalid(format!(
            "올바른 도메인 형식이 아닙니다(소문자 FQDN만): {}",
            if domain.is_empty() {
                "(빈 값)"
            } else {
                domain
            }
        )));
    }
    Ok(())
}

pub fn validate_domains(domains: &[String], allow_wildcard: bool) -> Result<()> {
    if domains.is_empty() {
        return Err(CertificateError::Invalid(
            "도메인을 하나 이상 입력하세요.".into(),
        ));
    }
    for domain in domains {
        validate_domain(domain, allow_wildcard)?;
    }
    Ok(())
}
