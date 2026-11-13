use nwui_api::certificates::{
    pem::{create_csr, parse_certificate_pem},
    validation::validate_domain,
};
use x509_parser::public_key::PublicKey;
use x509_parser::{certification_request::X509CertificationRequest, prelude::*};

#[test]
fn domain_validation_preserves_dns_wildcards_and_rejects_malformed_labels() {
    assert!(validate_domain("*.example.com", true).is_ok());
    assert!(validate_domain("a-b.sub.example.co.kr", false).is_ok());
    assert!(
        validate_domain("*.example.com", false)
            .unwrap_err()
            .to_string()
            .contains("DNS-01")
    );
    for name in [
        "_",
        "localhost",
        "192.168.0.1",
        "Example.com",
        "a\n.com",
        "*.*.example.com",
        "",
        "-bad.com",
    ] {
        assert!(validate_domain(name, true).is_err(), "{name}");
    }
    assert!(validate_domain(&format!("{}.com", "a".repeat(64)), true).is_err());
    assert!(validate_domain(&format!("example.{}", "a".repeat(64)), true).is_err());
}

#[test]
#[ignore = "golden vectors: set NWUI_CONTROL_FIXTURES"]
fn generated_csr_matches_legacy_semantics_and_has_valid_rsa_signature() {
    let fixtures: serde_json::Value = serde_json::from_slice(&std::fs::read(std::path::Path::new(&std::env::var("NWUI_CONTROL_FIXTURES").expect("set NWUI_CONTROL_FIXTURES")).join("acme/vectors.json")).expect("golden fixture"))
    .unwrap();
    for case in fixtures["csr"]["cases"].as_array().unwrap() {
        let names: Vec<String> = serde_json::from_value(case["input"]["altNames"].clone()).unwrap();
        let material = create_csr(&names).unwrap();
        let (_, pem) = x509_parser::pem::parse_x509_pem(material.csr_pem.as_bytes()).unwrap();
        let (_, csr) = X509CertificationRequest::from_der(&pem.contents).unwrap();
        assert_eq!(csr.certification_request_info.subject.to_string(), "");
        assert_eq!(
            csr.signature_algorithm.algorithm.to_id_string(),
            "1.2.840.113549.1.1.11"
        );
        csr.verify_signature().unwrap();
        let extensions = csr
            .certification_request_info
            .iter_attributes()
            .find_map(|a| match a.parsed_attribute() {
                x509_parser::cri_attributes::ParsedCriAttribute::ExtensionRequest(r) => {
                    Some(&r.extensions)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(extensions.len(), 2);
        let ku = extensions
            .iter()
            .find(|e| e.oid.to_id_string() == "2.5.29.15")
            .unwrap();
        assert!(!ku.critical);
        assert!(
            matches!(ku.parsed_extension(), ParsedExtension::KeyUsage(k) if k.digital_signature() && k.key_encipherment())
        );
        let san = extensions
            .iter()
            .find(|e| e.oid.to_id_string() == "2.5.29.17")
            .unwrap();
        let ParsedExtension::SubjectAlternativeName(san) = san.parsed_extension() else {
            panic!("SAN");
        };
        let actual: Vec<&str> = san
            .general_names
            .iter()
            .map(|n| match n {
                GeneralName::DNSName(s) => *s,
                _ => panic!("DNS SAN"),
            })
            .collect();
        assert_eq!(actual, names);
        let PublicKey::RSA(rsa) = csr.certification_request_info.subject_pki.parsed().unwrap()
        else {
            panic!("RSA")
        };
        assert_eq!(rsa.key_size(), 2048);
    }
}

#[test]
fn imported_certificate_checks_private_key_and_prefers_san_names() {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params =
        rcgen::CertificateParams::new(vec!["EXAMPLE.com".to_owned(), "example.com".to_owned()])
            .unwrap();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "ignored.example");
    let cert = params.self_signed(&key).unwrap();
    let info = parse_certificate_pem(&cert.pem(), &key.serialize_pem()).unwrap();
    assert_eq!(info.domains, vec!["example.com"]);
    assert!(info.not_after > info.not_before);
    assert!(
        parse_certificate_pem(
            &cert.pem(),
            &rcgen::KeyPair::generate().unwrap().serialize_pem()
        )
        .unwrap_err()
        .to_string()
        .contains("일치")
    );
    assert!(
        parse_certificate_pem(&cert.pem(), "-----BEGIN ENCRYPTED PRIVATE KEY-----")
            .unwrap_err()
            .to_string()
            .contains("암호화")
    );
}

#[test]
#[ignore = "golden vectors: set NWUI_CONTROL_FIXTURES"]
fn existing_rsa_pem_import_vectors_remain_compatible() {
    let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(std::path::Path::new(&std::env::var("NWUI_CONTROL_FIXTURES").expect("set NWUI_CONTROL_FIXTURES")).join("nginx/parse-pem.json")).expect("golden fixture"))
    .unwrap();
    for vector in fixture["vectors"].as_array().unwrap() {
        let result = parse_certificate_pem(
            vector["args"][0].as_str().unwrap(),
            vector["args"][1].as_str().unwrap(),
        );
        if let Some(message) = vector["error"]["message"].as_str() {
            assert_eq!(result.unwrap_err().to_string(), message);
        } else {
            let actual = result.unwrap();
            assert_eq!(
                serde_json::to_value(&actual.domains).unwrap(),
                vector["result"]["domains"]
            );
            assert_eq!(
                actual.not_after,
                chrono::DateTime::parse_from_rfc3339(
                    vector["result"]["notAfter"]["$date"].as_str().unwrap()
                )
                .unwrap()
            );
        }
    }
}
