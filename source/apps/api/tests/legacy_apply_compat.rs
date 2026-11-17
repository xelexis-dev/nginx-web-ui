mod auth_support;
use auth_support::*;
use nwui_api::{contracts::Actor, control::{model, resources::{self, Mutation, Resource}}};
use serde_json::{Value,json};

fn cases() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/legacy-apply-inputs.json")).unwrap()
}

#[tokio::test]
#[ignore = "real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn legacy_stored_real_ip_and_targets_render_released_119_bytes() {
    let db=test_db().await;
    let mut failures=vec![];
    for case in cases() {
        db.client.batch_execute("DELETE FROM nginx_servers").await.unwrap();
        db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('legacy','legacy.test',18081,$1)", &[&case["config"]]).await.unwrap();
        let client=db.state.db.get().await.unwrap();
        let data=model::ConfigSnapshotData { servers:resources::list(&**client,Resource::Server).await.unwrap(),upstreams:vec![],streams:vec![] };drop(client);
        let rendered=nwui_api::publisher::test_support::render_for_apply(&db.state.config,&data,&[]);
        let actual=rendered.rendered.conf.get("server-legacy.conf").map(|b|String::from_utf8(b.clone()).unwrap());
        let expected=case["expected"].as_str().expect("released-render capture missing");
        if actual.as_deref()!=Some(expected) {
            failures.push(format!("{}: skipped={:?}, actual={actual:?}",case["name"],rendered.skipped));
        }
        assert_eq!(db.client.query_one("SELECT config FROM nginx_servers WHERE id='legacy'",&[]).await.unwrap().get::<_,Value>(0),case["config"]);
    }
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[tokio::test]
#[ignore = "real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn legacy_untouched_real_ip_and_targets_remain_editable_but_changes_are_strict() {
    let db=test_db().await;
    let actor=Actor{via: None,id:None,label:Some("legacy-save".into()),ip:None};
    let mut failures=vec![];
    for case in cases() {
        db.client.batch_execute("DELETE FROM nginx_servers").await.unwrap();
        let cfg=&case["config"];
        db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('legacy','legacy.test',18081,$1)", &[cfg]).await.unwrap();
        for multiple in [false,true] {
            let mut raw=json!({"serverName":"edited.test","listen":18081,"proxyPass":cfg["proxyPass"],"realIp":cfg["realIp"]});
            if multiple {
                raw["targetMode"]=json!("locations");
                raw["locations"]=json!([{"path":"/","mode":"direct","proxyPass":cfg["proxyPass"]}]);
            }
            let saved=resources::mutate(&db.state,Resource::Server,Mutation::Update,Some("legacy"),raw.clone(),&actor).await;
            match saved {
                Err(error)=>failures.push(format!("{} multiple={multiple}: {error:?}",case["name"])),
                Ok(row)=>{
                    assert_eq!(row["config"]["realIp"],cfg["realIp"],"stored realIp rewritten");
                    assert_eq!(row["config"]["locations"][0]["proxyPass"],cfg["proxyPass"]);
                }
            }
            let mut changed=raw.clone();
            if cfg.get("realIp").is_some() {
                changed["realIp"]=json!({"preset":"new-typo"});
            } else if multiple {
                changed["locations"][0]["proxyPass"]=json!(format!("{}changed",cfg["proxyPass"].as_str().unwrap()));
            } else {
                changed["proxyPass"]=json!(format!("{}changed",cfg["proxyPass"].as_str().unwrap()));
            }
            let before:Value=db.client.query_one("SELECT to_jsonb(s) FROM nginx_servers s WHERE id='legacy'",&[]).await.unwrap().get(0);
            let error=resources::mutate(&db.state,Resource::Server,Mutation::Update,Some("legacy"),changed,&actor).await.expect_err("new malformed value accepted");
            assert_eq!(error.status,400,"{error:?}");
            let field=if cfg.get("realIp").is_some(){"realIp"}else if multiple{"locations"}else{"proxyPass"};
            assert!(error.field_errors.as_ref().is_some_and(|fields| fields.contains_key(field)),"{error:?}");
            assert_eq!(db.client.query_one("SELECT to_jsonb(s) FROM nginx_servers s WHERE id='legacy'",&[]).await.unwrap().get::<_,Value>(0),before);
        }
    }
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}
