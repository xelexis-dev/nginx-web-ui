use crate::control::model::ConfigSnapshotData;
use anyhow::Result;
use serde_json::json;

pub(crate) async fn replace_rows(
    tx: &deadpool_postgres::Transaction<'_>,
    data: &ConfigSnapshotData,
) -> Result<()> {
    let names=super::location_refs::upstream_names(&data.upstreams);
    for row in &data.servers{super::location_refs::resolve_locations(&row["config"],&names)?;}
    for row in data.upstreams.iter().chain(&data.streams) {
        super::validation::parse_runtime_dns_flag(row)?;
    }
    for row in &data.servers {
        super::validation::parse_runtime_dns_flag(&row["config"])?;
        for location in row["config"]["locations"].as_array().into_iter().flatten() {
            super::validation::parse_runtime_dns_flag(location)?;
        }
    }
    super::render::validate_http_group_names(&data.upstreams, &data.servers)?;
    tx.batch_execute("DELETE FROM nginx_servers; DELETE FROM nginx_streams; DELETE FROM upstreams")
        .await?;
    tx.execute("INSERT INTO upstreams(id,name,servers,resolve_at_runtime) SELECT id,name,servers,COALESCE(\"resolveAtRuntime\",false) FROM jsonb_to_recordset($1::jsonb) AS x(id text,name text,servers jsonb,\"resolveAtRuntime\" boolean)",&[&json!(data.upstreams)]).await?;
    tx.execute("INSERT INTO nginx_servers(id,server_name,listen,upstream_id,config,enabled,certificate_id) SELECT id,\"serverName\",COALESCE(listen,80),\"upstreamId\",config,COALESCE(enabled,true),\"certificateId\" FROM jsonb_to_recordset($1::jsonb) AS x(id text,\"serverName\" text,listen integer,\"upstreamId\" text,config jsonb,enabled boolean,\"certificateId\" text)",&[&json!(data.servers)]).await?;
    tx.execute("INSERT INTO nginx_streams(id,listen,protocol,proxy_pass,enabled,resolve_at_runtime) SELECT id,listen,COALESCE(protocol,'tcp')::protocol,\"proxyPass\",COALESCE(enabled,true),COALESCE(\"resolveAtRuntime\",false) FROM jsonb_to_recordset($1::jsonb) AS x(id text,listen integer,protocol text,\"proxyPass\" text,enabled boolean,\"resolveAtRuntime\" boolean)",&[&json!(data.streams)]).await?;
    Ok(())
}
#[cfg(test)]
#[path="g13_resources_kernel_tests.rs"]
mod g13_resources;
