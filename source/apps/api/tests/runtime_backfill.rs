mod auth_support;
use nwui_api::runtime_backfill::{self, BackfillResult};
use serde_json::{Value, json};

async fn legacy() -> auth_support::TestDb {
    let t = auth_support::test_db().await;
    t.client.execute("INSERT INTO certificates(id,domains,key_pem,dns_order) VALUES('legacy','[\"example.com\"]',$1,$2),('empty','[]','',NULL),('sealed','[]','enc:v1:broken',$3)", &[&"private-cert",&json!({"keyPem":"private-order","orderUrl":"https://ca.test/order/1","nested":{"keep":true}}),&json!({"keyPem":"enc:v1:broken","keep":7})]).await.unwrap();
    t.client.execute("INSERT INTO acme_accounts(id,directory_url,email,account_key_pem) VALUES('account','https://ca.test','a@example.com','private-account')",&[]).await.unwrap();
    auth_support::insert_user(&t.client, auth_support::UserRow::default(), None).await;
    t.client
        .execute(
            "UPDATE users SET totp_secret='private-totp' WHERE id='u1'",
            &[],
        )
        .await
        .unwrap();
    t
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn only_exact_opt_in_changes_legacy_values() {
    let t = legacy().await;
    for value in [None, Some("0"), Some("true"), Some(" 1")] {
        assert_eq!(
            runtime_backfill::run_if_requested(&t.state, value).await,
            None
        );
    }
    assert_eq!(
        t.client
            .query_one("SELECT key_pem FROM certificates WHERE id='legacy'", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "private-cert"
    );
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn seals_four_legacy_fields_and_preserves_existing_values_and_order() {
    let t = legacy().await;
    let result = runtime_backfill::run_if_requested(&t.state, Some("1"))
        .await
        .unwrap();
    assert_eq!(
        result,
        BackfillResult {
            cert_keys: 1,
            account_keys: 1,
            totp_secrets: 1,
            dns_orders: 1,
            failed: 0
        }
    );
    let row = t
        .client
        .query_one(
            "SELECT key_pem,dns_order FROM certificates WHERE id='legacy'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        t.state
            .secrets
            .open(Some(&row.get::<_, String>(0)))
            .as_deref(),
        Some("private-cert")
    );
    let mut order: Value = row.get(1);
    assert_eq!(
        t.state.secrets.open(order["keyPem"].as_str()).as_deref(),
        Some("private-order")
    );
    order["keyPem"] = json!("private-order");
    assert_eq!(
        order,
        json!({"keyPem":"private-order","orderUrl":"https://ca.test/order/1","nested":{"keep":true}})
    );
    for (query, plain) in [
        (
            "SELECT account_key_pem FROM acme_accounts",
            "private-account",
        ),
        ("SELECT totp_secret FROM users", "private-totp"),
    ] {
        let value: String = t.client.query_one(query, &[]).await.unwrap().get(0);
        assert_eq!(t.state.secrets.open(Some(&value)).as_deref(), Some(plain));
    }
    let row = t
        .client
        .query_one(
            "SELECT key_pem,dns_order FROM certificates WHERE id='sealed'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "enc:v1:broken");
    assert_eq!(
        row.get::<_, Value>(1),
        json!({"keyPem":"enc:v1:broken","keep":7})
    );
    assert_eq!(
        t.client
            .query_one("SELECT key_pem FROM certificates WHERE id='empty'", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        ""
    );
    assert_eq!(
        runtime_backfill::run_if_requested(&t.state, Some("1")).await,
        Some(BackfillResult::default())
    );
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn concurrent_runs_do_not_double_seal() {
    let t = legacy().await;
    let (a, b) = tokio::join!(
        runtime_backfill::run_if_requested(&t.state, Some("1")),
        runtime_backfill::run_if_requested(&t.state, Some("1"))
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(
        (
            a.cert_keys + b.cert_keys,
            a.account_keys + b.account_keys,
            a.totp_secrets + b.totp_secrets,
            a.dns_orders + b.dns_orders,
            a.failed + b.failed
        ),
        (1, 1, 1, 1, 0)
    );
    let key: String = t
        .client
        .query_one("SELECT key_pem FROM certificates WHERE id='legacy'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        t.state.secrets.open(Some(&key)).as_deref(),
        Some("private-cert")
    );
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn failed_group_rolls_back_and_other_groups_remain_available() {
    let t = legacy().await;
    t.client.batch_execute("INSERT INTO certificates(id,domains,key_pem) VALUES('z-failing','[]','private-failing'); CREATE FUNCTION reject_backfill() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.id='z-failing' THEN RAISE EXCEPTION 'private-error-must-never-be-reported'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_backfill BEFORE UPDATE ON certificates FOR EACH ROW EXECUTE FUNCTION reject_backfill();").await.unwrap();
    let result = runtime_backfill::run_if_requested(&t.state, Some("1"))
        .await
        .unwrap();
    assert_eq!(
        result,
        BackfillResult {
            cert_keys: 0,
            dns_orders: 0,
            account_keys: 1,
            totp_secrets: 1,
            failed: 1
        }
    );
    assert_eq!(
        t.client
            .query_one("SELECT key_pem FROM certificates WHERE id='legacy'", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "private-cert"
    );
    assert!(!format!("{result:?}").contains("private"));
    t.client
        .batch_execute("DROP TRIGGER reject_backfill ON certificates")
        .await
        .unwrap();
    assert_eq!(
        runtime_backfill::run_if_requested(&t.state, Some("1"))
            .await
            .unwrap(),
        BackfillResult {
            cert_keys: 2,
            dns_orders: 1,
            ..Default::default()
        }
    );
}
