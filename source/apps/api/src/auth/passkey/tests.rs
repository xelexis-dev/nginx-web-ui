use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use serde_json::{Value, json};

fn vector(algorithm: &str) -> Value {
    serde_json::from_str(match algorithm {
        "es256" => include_str!("../../../tests/fixtures/passkey/es256.json"),
        "rs256" => include_str!("../../../tests/fixtures/passkey/rs256.json"),
        _ => unreachable!(),
    })
    .unwrap()
}
fn bytes(v: &Value) -> Vec<u8> {
    B64.decode(v.as_str().unwrap()).unwrap()
}
fn expected<'a>(v: &'a Value, challenge: &'a [u8]) -> Expected<'a> {
    Expected {
        rp_id: v["rpId"].as_str().unwrap(),
        origin: v["origin"].as_str().unwrap(),
        challenge,
    }
}
fn registration(v: &Value) -> Result<VerifiedRegistration, PasskeyError> {
    let challenge = bytes(&v["challenge"]);
    verify_registration(
        &parse_registration(&v["registration"].to_string())?,
        &expected(v, &challenge),
    )
}
fn assertion(v: &Value) -> Result<VerifiedAssertion, PasskeyError> {
    let reg = registration(v)?;
    let challenge = bytes(&v["assertionChallenge"]);
    let handle = bytes(&v["userHandle"]);
    verify_assertion(
        &parse_assertion(&v["assertion"].to_string())?,
        &expected(v, &challenge),
        &StoredCredential {
            public_key_spki: &reg.public_key_spki,
            algorithm: reg.algorithm,
            backup_eligible: reg.backup_eligible,
            rp_id: v["rpId"].as_str().unwrap(),
            origin: v["origin"].as_str().unwrap(),
            user_handle: &handle,
        },
    )
}
fn client(v: &mut Value, ceremony: &str, change: impl FnOnce(&mut Value)) {
    let field = &mut v[ceremony]["response"]["clientDataJSON"];
    let mut data: Value = serde_json::from_slice(&bytes(field)).unwrap();
    change(&mut data);
    *field = json!(B64.encode(serde_json::to_vec(&data).unwrap()));
}
fn auth(v: &mut Value, ceremony: &str, change: impl FnOnce(&mut Vec<u8>)) {
    let field = &mut v[ceremony]["response"]["authenticatorData"];
    let mut data = bytes(field);
    change(&mut data);
    *field = json!(B64.encode(data));
}
macro_rules! refuses_registration {
    ($name:ident, $error:ident, $change:expr) => {
        #[test]
        fn $name() {
            let mut v = vector("es256");
            ($change)(&mut v);
            assert_eq!(registration(&v).unwrap_err(), PasskeyError::$error);
        }
    };
}
macro_rules! refuses_assertion {
    ($name:ident, $error:ident, $change:expr) => {
        #[test]
        fn $name() {
            let mut v = vector("es256");
            ($change)(&mut v);
            assert_eq!(assertion(&v).unwrap_err(), PasskeyError::$error);
        }
    };
}
#[test]
fn chromium_positive_vectors() {
    for alg in ["es256", "rs256"] {
        let v = vector(alg);
        let r = registration(&v).unwrap();
        let a = assertion(&v).unwrap();
        assert_eq!(
            r.algorithm,
            if alg == "es256" {
                PasskeyAlg::Es256
            } else {
                PasskeyAlg::Rs256
            }
        );
        assert!(counter_accepts(r.sign_count, a.sign_count));
        let input = parse_assertion(&v["assertion"].to_string()).unwrap();
        assert_eq!(input.raw_id(), r.credential_id);
        assert_eq!(
            input.user_handle(),
            Some(bytes(&v["userHandle"]).as_slice())
        );
    }
}
refuses_registration!(bad_json, InvalidInput, |v: &mut Value| v["registration"] =
    json!(null));
refuses_registration!(
    wrong_credential_type,
    CredentialType,
    |v: &mut Value| v["registration"]["type"] = json!("password")
);
refuses_registration!(
    bad_base64,
    InvalidEncoding,
    |v: &mut Value| v["registration"]["rawId"] = json!("%%%")
);
refuses_registration!(empty_id, CredentialId, |v: &mut Value| v["registration"]
    ["rawId"] =
    json!(""));
refuses_registration!(
    mismatched_id,
    CredentialIdMismatch,
    |v: &mut Value| v["registration"]["id"] = json!("AA")
);
refuses_registration!(
    id_too_long,
    FieldTooLarge,
    |v: &mut Value| v["registration"]["rawId"] = json!(B64.encode(vec![0; 1024]))
);
refuses_registration!(
    invalid_client_json,
    ClientData,
    |v: &mut Value| v["registration"]["response"]["clientDataJSON"] = json!(B64.encode(b"{"))
);
refuses_registration!(registration_type, ClientType, |v: &mut Value| client(
    v,
    "registration",
    |c| c["type"] = json!("webauthn.get")
));
refuses_assertion!(assertion_type, ClientType, |v: &mut Value| client(
    v,
    "assertion",
    |c| c["type"] = json!("webauthn.create")
));
refuses_registration!(registration_challenge, Challenge, |v: &mut Value| client(
    v,
    "registration",
    |c| c["challenge"] = json!(B64.encode([0; 32]))
));
refuses_assertion!(assertion_challenge, Challenge, |v: &mut Value| client(
    v,
    "assertion",
    |c| c["challenge"] = json!(B64.encode([0; 32]))
));
refuses_registration!(registration_origin, Origin, |v: &mut Value| client(
    v,
    "registration",
    |c| c["origin"] = json!("http://localhost:1")
));
refuses_assertion!(assertion_origin, Origin, |v: &mut Value| client(
    v,
    "assertion",
    |c| c["origin"] = json!("http://localhost:1")
));
refuses_registration!(registration_cross_origin, CrossOrigin, |v: &mut Value| {
    client(v, "registration", |c| c["crossOrigin"] = json!(true))
});
refuses_assertion!(assertion_cross_origin, CrossOrigin, |v: &mut Value| client(
    v,
    "assertion",
    |c| c["crossOrigin"] = json!(true)
));
refuses_registration!(cross_origin_null, CrossOrigin, |v: &mut Value| client(
    v,
    "registration",
    |c| c["crossOrigin"] = json!(null)
));
refuses_assertion!(
    cross_origin_wrong_type,
    CrossOrigin,
    |v: &mut Value| client(v, "assertion", |c| c["crossOrigin"] = json!("false"))
);
refuses_registration!(registration_top_origin, TopOrigin, |v: &mut Value| client(
    v,
    "registration",
    |c| c["topOrigin"] = json!(null)
));
refuses_assertion!(assertion_top_origin, TopOrigin, |v: &mut Value| client(
    v,
    "assertion",
    |c| c["topOrigin"] = json!("http://localhost")
));
refuses_registration!(short_auth_data, AuthData, |v: &mut Value| auth(
    v,
    "registration",
    |b| b.truncate(36)
));
refuses_registration!(registration_rp_hash, RpIdHash, |v: &mut Value| auth(
    v,
    "registration",
    |b| b[0] ^= 1
));
refuses_assertion!(assertion_rp_hash, RpIdHash, |v: &mut Value| auth(
    v,
    "assertion",
    |b| b[0] ^= 1
));
refuses_registration!(registration_up, UserPresence, |v: &mut Value| auth(
    v,
    "registration",
    |b| b[32] &= !1
));
refuses_assertion!(assertion_up, UserPresence, |v: &mut Value| auth(
    v,
    "assertion",
    |b| b[32] &= !1
));
refuses_registration!(registration_uv, UserVerification, |v: &mut Value| auth(
    v,
    "registration",
    |b| b[32] &= !4
));
refuses_assertion!(assertion_uv, UserVerification, |v: &mut Value| auth(
    v,
    "assertion",
    |b| b[32] &= !4
));
refuses_registration!(missing_at, AttestedData, |v: &mut Value| auth(
    v,
    "registration",
    |b| b[32] &= !0x40
));
refuses_registration!(registration_bad_backup, BackupFlags, |v: &mut Value| auth(
    v,
    "registration",
    |b| b[32] = (b[32] & !8) | 16
));
refuses_assertion!(assertion_bad_backup, BackupFlags, |v: &mut Value| auth(
    v,
    "assertion",
    |b| b[32] = (b[32] & !8) | 16
));
refuses_assertion!(be_changed, BackupEligibility, |v: &mut Value| auth(
    v,
    "assertion",
    |b| b[32] ^= 8
));
refuses_registration!(missing_attested_bytes, AuthData, |v: &mut Value| auth(
    v,
    "registration",
    |b| b.truncate(54)
));
refuses_registration!(overlong_attested_id, CredentialId, |v: &mut Value| auth(
    v,
    "registration",
    |b| {
        b[53] = 4;
        b[54] = 0;
    }
));
refuses_registration!(truncated_attested_id, AuthData, |v: &mut Value| auth(
    v,
    "registration",
    |b| b.truncate(56)
));
refuses_registration!(
    attested_id_mismatch,
    CredentialIdMismatch,
    |v: &mut Value| auth(v, "registration", |b| b[55] ^= 1)
);
refuses_registration!(
    unoffered_algorithm,
    UnsupportedAlgorithm,
    |v: &mut Value| v["registration"]["response"]["publicKeyAlgorithm"] = json!(-8)
);
refuses_registration!(missing_public_key, MissingPublicKey, |v: &mut Value| {
    v["registration"]["response"]
        .as_object_mut()
        .unwrap()
        .remove("publicKey");
});
refuses_registration!(
    bad_public_key,
    InvalidPublicKey,
    |v: &mut Value| v["registration"]["response"]["publicKey"] = json!("AA")
);
refuses_registration!(key_algorithm_mismatch, InvalidPublicKey, |v: &mut Value| {
    v["registration"]["response"]["publicKeyAlgorithm"] = json!(-257)
});
refuses_registration!(
    not_resident,
    NotDiscoverable,
    |v: &mut Value| v["registration"]["clientExtensionResults"]["credProps"]["rk"] = json!(false)
);
refuses_assertion!(missing_handle, UserHandle, |v: &mut Value| {
    v["assertion"]["response"]
        .as_object_mut()
        .unwrap()
        .remove("userHandle");
});
refuses_assertion!(
    wrong_handle,
    UserHandle,
    |v: &mut Value| v["assertion"]["response"]["userHandle"] = json!("AA")
);
refuses_assertion!(flipped_auth_signed_part, Signature, |v: &mut Value| auth(
    v,
    "assertion",
    |b| b[36] ^= 1
));
refuses_assertion!(flipped_client_signed_part, Origin, |v: &mut Value| {
    let f = &mut v["assertion"]["response"]["clientDataJSON"];
    let mut b = bytes(f);
    let pos = b.windows(7).position(|w| w == b"http://").unwrap();
    b[pos] ^= 1;
    *f = json!(B64.encode(b));
});
refuses_assertion!(flipped_signature, Signature, |v: &mut Value| {
    let f = &mut v["assertion"]["response"]["signature"];
    let mut b = bytes(f);
    b[0] ^= 1;
    *f = json!(B64.encode(b));
});
fn negative_key(name: &str) -> Value {
    let keys: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/passkey/negative-keys.json"
    ))
    .unwrap();
    keys[name].clone()
}
refuses_registration!(
    p384_refused,
    InvalidCurve,
    |v: &mut Value| v["registration"]["response"]["publicKey"] = negative_key("p384")
);
refuses_registration!(rsa1024_refused, WeakRsaKey, |v: &mut Value| {
    v["registration"]["response"]["publicKey"] = negative_key("rsa1024");
    v["registration"]["response"]["publicKeyAlgorithm"] = json!(-257);
});
refuses_assertion!(
    another_key_signature,
    Signature,
    |v: &mut Value| v["registration"]["response"]["publicKey"] = negative_key("otherEs256")
);

#[test]
fn stored_binding_refusals() {
    let v = vector("es256");
    let r = registration(&v).unwrap();
    let challenge = bytes(&v["assertionChallenge"]);
    let handle = bytes(&v["userHandle"]);
    let i = parse_assertion(&v["assertion"].to_string()).unwrap();
    let e = expected(&v, &challenge);
    let mut s = StoredCredential {
        public_key_spki: &r.public_key_spki,
        algorithm: r.algorithm,
        backup_eligible: r.backup_eligible,
        rp_id: e.rp_id,
        origin: e.origin,
        user_handle: &handle,
    };
    s.rp_id = "another.localhost";
    assert_eq!(
        verify_assertion(&i, &e, &s).unwrap_err(),
        PasskeyError::StoredRpId
    );
    s.rp_id = e.rp_id;
    s.origin = "http://localhost:1";
    assert_eq!(
        verify_assertion(&i, &e, &s).unwrap_err(),
        PasskeyError::StoredOrigin
    );
    // An otherwise valid signature on this port must fail against another allowed port.
    let another = Expected {
        rp_id: e.rp_id,
        origin: "http://localhost:1",
        challenge: &challenge,
    };
    assert_eq!(
        verify_assertion(&i, &another, &s).unwrap_err(),
        PasskeyError::Origin
    );
}
#[test]
fn counter_rules() {
    for (stored, received, accept) in [
        (10, 0, false),
        (5, 5, false),
        (0, 0, true),
        (0, 1, true),
        (5, 6, true),
        (6, 5, false),
        (u32::MAX, u32::MAX, false),
    ] {
        assert_eq!(counter_accepts(stored, received), accept);
    }
}
#[test]
fn parser_field_limits() {
    for ceremony in ["registration", "assertion"] {
        for (field, limit) in [
            ("clientDataJSON", 4096),
            ("authenticatorData", 16384),
            ("signature", 1024),
            ("userHandle", 64),
            ("publicKey", 2048),
            ("attestationObject", 17408),
        ] {
            if (ceremony == "registration" && matches!(field, "signature" | "userHandle"))
                || (ceremony == "assertion" && matches!(field, "publicKey" | "attestationObject"))
            {
                continue;
            }
            let mut v = vector("es256");
            v[ceremony]["response"][field] = json!(B64.encode(vec![0; limit + 1]));
            let j = v[ceremony].to_string();
            let error = if ceremony == "registration" {
                parse_registration(&j).err()
            } else {
                parse_assertion(&j).err()
            };
            assert_eq!(
                error,
                Some(PasskeyError::FieldTooLarge),
                "{ceremony}/{field}"
            );
        }
    }
    assert_eq!(
        parse_registration(&" ".repeat(65537)).err(),
        Some(PasskeyError::FieldTooLarge)
    );
    let mut v = vector("es256");
    v["registration"]["authenticatorAttachment"] = json!("x".repeat(33));
    assert_eq!(
        parse_registration(&v["registration"].to_string()).err(),
        Some(PasskeyError::FieldTooLarge)
    );
    for transports in [
        json!(["x".repeat(33)]),
        json!(["a", "b", "c", "d", "e", "f", "g", "h", "i"]),
    ] {
        let mut v = vector("es256");
        v["registration"]["response"]["transports"] = transports;
        assert_eq!(
            parse_registration(&v["registration"].to_string()).err(),
            Some(PasskeyError::FieldTooLarge)
        );
    }
}
#[test]
fn absent_cross_origin_and_cred_props_allowed() {
    let mut v = vector("es256");
    client(&mut v, "registration", |c| {
        c.as_object_mut().unwrap().remove("crossOrigin");
    });
    v["registration"]
        .as_object_mut()
        .unwrap()
        .remove("clientExtensionResults");
    registration(&v).unwrap();
}
#[test]
fn availability_matrix() {
    use UnavailableReason::*;
    for (origin, secure, configured, reason) in [
        ("http://192.0.2.1:8080", false, false, Some(IpHost)),
        ("https://[::1]", true, false, Some(IpHost)),
        ("http://localhost:8080", false, false, None),
        ("http://nas.local:8080", false, false, Some(InsecureOrigin)),
        ("https://nas.local:8443", true, false, None),
        ("https://nas.local", false, false, Some(InsecureOrigin)),
        ("https://nas.local:8443", false, true, None),
        ("http://nas.local", true, true, Some(AuthUrlHttp)),
        ("http://localhost:8080", false, true, None),
        ("https://127.0.0.1", true, true, Some(IpHost)),
        ("https://0x7f000001", true, true, Some(IpHost)),
        ("https://2130706433", true, false, Some(IpHost)),
        ("https://nas.local.", true, false, Some(InsecureOrigin)),
        ("https://nas.local.", true, true, Some(InsecureOrigin)),
        ("https://localhost.", true, true, Some(InsecureOrigin)),
        ("ftp://localhost", true, false, Some(InsecureOrigin)),
        ("garbage", true, true, Some(InsecureOrigin)),
    ] {
        let ctx = crate::request_context::RequestContext {
            ip: None,
            external_origin: origin.into(),
            secure,
            origin_configured: configured,
        };
        let result = availability(&ctx, configured.then_some(origin));
        match reason {
            Some(reason) => assert_eq!(result, Availability::Unavailable(reason), "{origin}"),
            None => {
                let url = url::Url::parse(origin).unwrap();
                assert_eq!(
                    result,
                    Availability::Available {
                        rp_id: url.host_str().unwrap().into(),
                        origin: url.origin().ascii_serialization()
                    },
                    "{origin}"
                );
            }
        }
    }
    let ctx = crate::request_context::RequestContext {
        ip: None,
        external_origin: "http://elsewhere.local".into(),
        secure: false,
        origin_configured: false,
    };
    assert_eq!(
        availability(&ctx, Some("https://auth.local:8443/path")),
        Availability::Available {
            rp_id: "auth.local".into(),
            origin: "https://auth.local:8443".into()
        }
    );
}

#[test]
fn chromium_counter_six_vectors() {
    let v = vector("es256");
    let r = registration(&v).unwrap();
    let handle = bytes(&v["userHandle"]);
    let stored = StoredCredential {
        public_key_spki: &r.public_key_spki,
        algorithm: r.algorithm,
        backup_eligible: r.backup_eligible,
        rp_id: v["rpId"].as_str().unwrap(),
        origin: v["origin"].as_str().unwrap(),
        user_handle: &handle,
    };
    let assertions = v["counterAssertions"].as_array().unwrap();
    assert_eq!(assertions.len(), 2);
    assert_ne!(assertions[0]["challenge"], assertions[1]["challenge"]);
    for a in assertions {
        let challenge = bytes(&a["challenge"]);
        let e = expected(a, &challenge);
        let verified = verify_assertion(
            &parse_assertion(&a["assertion"].to_string()).unwrap(),
            &e,
            &stored,
        )
        .unwrap();
        assert_eq!(verified.sign_count, 6);
        assert_eq!(a["signCount"], 6);
        assert_eq!(bytes(&a["userHandle"]), handle);
        assert!(counter_accepts(5, verified.sign_count));
        assert!(!counter_accepts(6, verified.sign_count));
    }
}

refuses_assertion!(changed_client_signed_part, Signature, |v: &mut Value| {
    client(v, "assertion", |c| c["extra"] = json!("changed"))
});

#[test]
fn registration_requires_bytes_after_credential_id() {
    for alg in ["es256", "rs256"] {
        let mut v = vector(alg);
        auth(&mut v, "registration", |b| {
            let length = u16::from_be_bytes([b[53], b[54]]) as usize;
            b.truncate(55 + length);
        });
        assert_eq!(registration(&v).unwrap_err(), PasskeyError::AuthData);
    }
}

// Local test keys sign malformed data only; positive evidence stays the Chromium fixtures.
fn signed_framing_refusal(change: fn(&mut Vec<u8>)) {
    use aws_lc_rs::{
        encoding::{AsDer, PublicKeyX509Der},
        rand::SystemRandom,
        rsa::KeySize,
        signature::{self, KeyPair},
    };
    use sha2::{Digest, Sha256};
    for name in ["es256", "rs256"] {
        let v = vector(name);
        let registration = registration(&v).unwrap();
        let mut input = parse_assertion(&v["assertion"].to_string()).unwrap();
        change(&mut input.auth_data);
        let mut message = input.auth_data.clone();
        message.extend_from_slice(&Sha256::digest(&input.client_data));
        let (spki, sig) = if name == "es256" {
            let key = signature::EcdsaKeyPair::generate(&signature::ECDSA_P256_SHA256_ASN1_SIGNING)
                .unwrap();
            (
                AsDer::<PublicKeyX509Der>::as_der(key.public_key())
                    .unwrap()
                    .as_ref()
                    .to_vec(),
                key.sign(&SystemRandom::new(), &message)
                    .unwrap()
                    .as_ref()
                    .to_vec(),
            )
        } else {
            let key = signature::RsaKeyPair::generate(KeySize::Rsa2048).unwrap();
            let mut sig = vec![0; key.public_modulus_len()];
            key.sign(
                &signature::RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                &message,
                &mut sig,
            )
            .unwrap();
            (
                AsDer::<PublicKeyX509Der>::as_der(key.public_key())
                    .unwrap()
                    .as_ref()
                    .to_vec(),
                sig,
            )
        };
        // Assert the signature independently, so Signature cannot mask a missing framing guard.
        let verification: &dyn signature::VerificationAlgorithm = if name == "es256" {
            &signature::ECDSA_P256_SHA256_ASN1
        } else {
            &signature::RSA_PKCS1_2048_8192_SHA256
        };
        signature::UnparsedPublicKey::new(verification, &spki)
            .verify(&message, &sig)
            .unwrap();
        input.signature = sig;
        let challenge = bytes(&v["assertionChallenge"]);
        let handle = bytes(&v["userHandle"]);
        let expected = expected(&v, &challenge);
        let stored = StoredCredential {
            public_key_spki: &spki,
            algorithm: registration.algorithm,
            backup_eligible: registration.backup_eligible,
            rp_id: expected.rp_id,
            origin: expected.origin,
            user_handle: &handle,
        };
        assert_eq!(
            verify_assertion(&input, &expected, &stored).unwrap_err(),
            PasskeyError::AuthData
        );
    }
}
#[test]
fn signed_assertion_at_is_refused() {
    signed_framing_refusal(|b| b[32] |= 0x40);
}
#[test]
fn signed_assertion_trailing_bytes_are_refused() {
    signed_framing_refusal(|b| b.push(0));
}
#[test]
fn signed_assertion_ed_without_bytes_is_refused() {
    signed_framing_refusal(|b| b[32] |= 0x80);
}
#[test]
fn signed_assertion_ed_with_bytes_is_refused() {
    signed_framing_refusal(|b| {
        b[32] |= 0x80;
        b.push(0xa0);
    });
}

#[test]
fn known_envelope_members_reject_wrong_types_on_both_fixtures() {
    for alg in ["es256", "rs256"] {
        for ceremony in ["registration", "assertion"] {
            let original = vector(alg);
            let c = &original[ceremony];
            let root_sequence = json!([
                c["id"],
                c["rawId"],
                c["type"],
                c["response"],
                c["authenticatorAttachment"],
                c["clientExtensionResults"]
            ]);
            assert_eq!(
                if ceremony == "registration" {
                    parse_registration(&root_sequence.to_string()).err()
                } else {
                    parse_assertion(&root_sequence.to_string()).err()
                },
                Some(PasskeyError::InvalidInput)
            );
            let mut sequence_response = vector(alg);
            let r = &original[ceremony]["response"];
            sequence_response[ceremony]["response"] = json!([
                r["clientDataJSON"],
                r["authenticatorData"],
                original["registration"]["response"]["publicKey"],
                original["registration"]["response"]["publicKeyAlgorithm"],
                "AA",
                "AA",
                ["internal"],
                "AA"
            ]);
            assert_eq!(
                if ceremony == "registration" {
                    parse_registration(&sequence_response[ceremony].to_string()).err()
                } else {
                    parse_assertion(&sequence_response[ceremony].to_string()).err()
                },
                Some(PasskeyError::InvalidInput)
            );
            let paths = [
                "/id",
                "/rawId",
                "/type",
                "/response",
                "/authenticatorAttachment",
                "/clientExtensionResults",
                "/response/clientDataJSON",
                "/response/authenticatorData",
                "/response/publicKey",
                "/response/publicKeyAlgorithm",
                "/response/signature",
                "/response/userHandle",
                "/response/transports",
                "/response/attestationObject",
                "/clientExtensionResults/credProps",
                "/clientExtensionResults/credProps/rk",
            ];
            for path in paths {
                let wrong_values = match path {
                    "/response"
                    | "/clientExtensionResults"
                    | "/clientExtensionResults/credProps" => [
                        Value::Null,
                        json!(17),
                        json!([]),
                        json!("object"),
                        json!(true),
                    ],
                    "/response/publicKeyAlgorithm" => {
                        [Value::Null, json!("-7"), json!([]), json!({}), json!(true)]
                    }
                    "/response/transports" => [
                        Value::Null,
                        json!(17),
                        json!({}),
                        json!("internal"),
                        json!(true),
                    ],
                    "/clientExtensionResults/credProps/rk" => {
                        [Value::Null, json!(17), json!([]), json!({}), json!("false")]
                    }
                    _ => [Value::Null, json!(17), json!([]), json!({}), json!(true)],
                };
                for wrong in wrong_values {
                    let mut v = vector(alg);
                    let object = &mut v[ceremony];
                    let (parent, field) = path.rsplit_once('/').unwrap();
                    if parent == "/clientExtensionResults/credProps" {
                        object["clientExtensionResults"]["credProps"] = json!({});
                    }
                    let parent = if parent.is_empty() {
                        object
                    } else {
                        object.pointer_mut(parent).unwrap()
                    };
                    parent[field] = wrong;
                    let result = if ceremony == "registration" {
                        parse_registration(&v[ceremony].to_string()).err()
                    } else {
                        parse_assertion(&v[ceremony].to_string()).err()
                    };
                    assert_eq!(
                        result,
                        Some(PasskeyError::InvalidInput),
                        "{alg}/{ceremony}{path}"
                    );
                }
            }
            let mut v = vector(alg);
            v[ceremony]["response"]["transports"] = json!([true]);
            assert_eq!(
                if ceremony == "registration" {
                    parse_registration(&v[ceremony].to_string()).err()
                } else {
                    parse_assertion(&v[ceremony].to_string()).err()
                },
                Some(PasskeyError::InvalidInput)
            );
        }
    }
}
#[test]
fn optional_and_unknown_members_stay_extensible() {
    for alg in ["es256", "rs256"] {
        let mut v = vector(alg);
        for ceremony in ["registration", "assertion"] {
            v[ceremony]["futureMember"] = json!({"nested":true});
            v[ceremony]["response"]["futureMember"] = json!([false]);
            v[ceremony]["clientExtensionResults"]["futureExtension"] = json!({"arbitrary":null});
            v[ceremony]["clientExtensionResults"]["credProps"] = json!({"futureProperty":true});
            v[ceremony]
                .as_object_mut()
                .unwrap()
                .remove("authenticatorAttachment");
        }
        registration(&v).unwrap();
        assertion(&v).unwrap();
    }
}
