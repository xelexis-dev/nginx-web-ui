use nwui_api::{
    certificates::service::{form_input, parse_input, parse_pem_input},
    contracts::{FormInput, Patch},
};
use serde_json::json;

#[test]
fn certificate_alias_omitted_null_and_value_remain_distinct() {
    let base =
        json!({"domains":[" EXAMPLE.com ",""],"email":" Admin@Example.com ","futureField":true});
    let value = parse_input(&base).unwrap();
    assert_eq!(value.domains, vec!["example.com"]);
    assert_eq!(value.email, "admin@example.com");
    assert_eq!(value.alias, Patch::Missing);
    let mut input = base.clone();
    input["alias"] = json!(null);
    assert_eq!(parse_input(&input).unwrap().alias, Patch::Null);
    input["alias"] = json!("   ");
    assert_eq!(parse_input(&input).unwrap().alias, Patch::Null);
    input["alias"] = json!(" 운영 ");
    assert_eq!(
        parse_input(&input).unwrap().alias,
        Patch::Value("운영".into())
    );
    input["alias"] = json!("x".repeat(101));
    assert!(
        parse_input(&input)
            .unwrap_err()
            .field_errors
            .unwrap()
            .contains_key("alias")
    );
}
#[test]
fn unknown_challenge_and_pem_size_are_rejected_with_field_errors() {
    let e=parse_input(&json!({"domains":["*.example.com"],"email":"admin@example.com","challengeType":"tls-alpn-01"})).unwrap_err();
    let fields = e.field_errors.unwrap();
    assert!(fields.contains_key("challengeType"));
    assert!(!fields.contains_key("domains"));
    let e = parse_pem_input(&json!({"certPem":"x".repeat(65537),"keyPem":""}))
        .err()
        .unwrap();
    assert_eq!(e.field_errors.unwrap().len(), 2);
}
#[test]
fn browser_form_omission_does_not_erase_existing_alias() {
    let mut input = FormInput::default();
    input.fields.insert(
        "domains".into(),
        vec!["example.com\r\nwww.example.com".into()],
    );
    input
        .fields
        .insert("email".into(), vec!["admin@example.com".into()]);
    assert!(form_input(&input).get("alias").is_none());
    input.fields.insert("alias".into(), vec!["".into()]);
    assert_eq!(parse_input(&form_input(&input)).unwrap().alias, Patch::Null);
}
#[test]
fn present_non_string_challenge_and_provider_are_not_silently_defaulted() {
    let input = json!({"domains":["example.com"],"email":"admin@example.com","challengeType":false,"dnsProvider":42});
    let fields = parse_input(&input).unwrap_err().field_errors.unwrap();
    assert!(fields.contains_key("challengeType"));
    assert!(fields.contains_key("dnsProvider"));
}
