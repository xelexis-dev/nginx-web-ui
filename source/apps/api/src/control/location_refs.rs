//! Location IDs stay canonical; names and URLs exist only in response/render copies.
use anyhow::{Context,Result};
use serde_json::{Value,json};
use std::collections::BTreeMap;

pub fn upstream_names(rows:&[Value])->BTreeMap<String,String>{
    rows.iter().filter_map(|row|Some((row["id"].as_str()?.to_owned(),row["name"].as_str()?.to_owned()))).collect()
}
pub fn resolve_locations(config:&Value,upstreams:&BTreeMap<String,String>)->Result<Value>{
    let mut resolved=config.clone();
    for location in resolved.get_mut("locations").and_then(Value::as_array_mut).into_iter().flatten(){
        if let Some(id)=location.get("upstreamId").filter(|v|!v.is_null()){
            let name=id.as_str().and_then(|id|upstreams.get(id)).context("이 설정이 쓰는 upstream을 찾을 수 없습니다")?;
            location["proxyPass"]=json!(format!("http://{name}"));
        }
    }
    Ok(resolved)
}
pub fn project_for_read(row:&Value,upstreams:&BTreeMap<String,String>)->Value{
    let mut projected=row.clone();
    for location in projected["config"].get_mut("locations").and_then(Value::as_array_mut).into_iter().flatten(){
        let name=location["upstreamId"].as_str().and_then(|id|upstreams.get(id)).cloned();
        if let Some(object)=location.as_object_mut(){object.remove("upstreamName");if let Some(name)=name{object.insert("upstreamName".into(),json!(name));}}
    }
    projected
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn pf_a1_resolution_and_projection_never_decorate_canonical_rows(){
        let row=json!({"config":{"locations":[{"path":"/","upstreamId":"u"},{"path":"/direct","proxyPass":"http://elsewhere/"}]}});let original=row.clone();
        for name in ["backend","renamed"]{
            let names=BTreeMap::from([("u".into(),name.into())]);let projected=project_for_read(&row,&names);assert_eq!(projected["config"]["locations"][0]["upstreamName"],name);assert!(projected["config"]["locations"][0].get("proxyPass").is_none());
            let resolved=resolve_locations(&row["config"],&names).unwrap();assert_eq!(resolved["locations"][0]["proxyPass"],format!("http://{name}"));assert_eq!(resolved["locations"][1],row["config"]["locations"][1]);
        }
        assert_eq!(project_for_read(&row,&BTreeMap::new()),row);assert!(resolve_locations(&row["config"],&BTreeMap::new()).is_err());assert_eq!(row,original);
    }


}
