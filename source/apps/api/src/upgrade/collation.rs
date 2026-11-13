//! Q18 auto-path admission, separate from the immutable A.22 schema fingerprints.
use super::StepError;
use crate::db_upgrade::cluster::Snapshot;
fn refused()->StepError{StepError{code:"collation_unsupported",message:"자동 업그레이드는 libc C/POSIX 정렬만 지원합니다. 다른 정렬을 사용하는 원본은 변경하지 않았습니다. 1.19.11에서 정렬 구성을 확인하세요.",detail:None}}
pub(crate) fn validate(snapshot:&Snapshot)->Result<(),StepError>{
    let stable=|v:&str|matches!(v,"C"|"POSIX");
    if snapshot.databases.is_empty()||snapshot.collations.len()!=snapshot.databases.len(){return Err(refused());}
    for db in &snapshot.databases{
        if db.provider!="c"||!stable(&db.collate)||!stable(&db.ctype){return Err(refused());}
        let used=snapshot.collations.get(&db.name).ok_or_else(refused)?;
        for c in used{match c.provider.as_str(){"d"=>{},"c" if c.locale.as_deref().is_some_and(stable)=>{},_=>return Err(refused())}}
    }Ok(())
}

#[cfg(test)]
mod tests{
    use super::*;
    use crate::db_upgrade::{catalog::Database,cluster::Collation,inspect,postgres};
    use std::collections::BTreeMap;
    fn snapshot()->Snapshot{Snapshot{databases:vec![Database{name:"postgres".into(),owner:"postgres".into(),encoding:"UTF8".into(),collate:"C".into(),ctype:"C".into(),provider:"c".into(),icu:None}],roles:vec![],data:BTreeMap::new(),collations:BTreeMap::from([("postgres".into(),vec![])]),largest_index_bytes:0}}
    fn libc(locale:&str)->Collation{Collation{schema:"pg_catalog".into(),name:locale.into(),provider:"c".into(),locale:Some(locale.into()),recorded:None,actual:None}}
    #[test]
    fn q18_database_defaults_reject_icu_non_c_ctype_and_c_utf8(){
        for case in ["icu","en_US.UTF-8","C.UTF-8","ctype"]{let mut s=snapshot();match case{"icu"=>{s.databases[0].provider="i".into();s.databases[0].icu=Some("en-US".into());},"ctype"=>s.databases[0].ctype="en_US.UTF-8".into(),_=>s.databases[0].collate=case.into()};assert_eq!(validate(&s).unwrap_err().code,"collation_unsupported","{case}");}
    }
    #[test]
    fn q18_used_named_and_default_collations_are_strict_and_collection_is_complete(){
        let mut s=snapshot();s.collations.get_mut("postgres").unwrap().extend([libc("C"),libc("POSIX")]);validate(&s).unwrap();s.collations.get_mut("postgres").unwrap().push(Collation{provider:"d".into(),locale:None,..libc("default")});validate(&s).unwrap();
        for provider in ["i","b","unknown"]{let mut s=snapshot();s.collations.get_mut("postgres").unwrap().push(Collation{provider:provider.into(),..libc("C")});assert!(validate(&s).is_err());}
        let mut s=snapshot();s.collations.get_mut("postgres").unwrap().push(libc("en_US.UTF-8"));assert!(validate(&s).is_err());
        s.collations.clear();assert!(validate(&s).is_err());
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="actual pinned PG15 collation fixture; NWUI_TEST_G2=1"]
    async fn native_q18_collects_unindexed_column_partition_and_domain_dependencies(){
        if std::env::var("NWUI_TEST_G2").as_deref()!=Ok("1"){eprintln!("NWUI_TEST_G2 not enabled");return;}
        let t=tempfile::tempdir().unwrap();use std::os::unix::fs::PermissionsExt;std::fs::set_permissions(t.path(),std::fs::Permissions::from_mode(0o711)).unwrap();let data=t.path().join("data");let log=t.path().join("logs");postgres::directory(&data).unwrap();postgres::directory(&log).unwrap();let mut init=postgres::command(15,"initdb").unwrap();init.arg("-D").arg(&data).args(["--no-locale","--encoding=UTF8","--auth=trust"]);postgres::capture(init,&log,std::time::Duration::from_secs(120)).unwrap();let pg=postgres::Private::start(15,&data,&log).unwrap();let c=inspect::connect(&pg.socket,"postgres","postgres",None).await.unwrap();
        let before=inspect::snapshot_database(&c.client,None,false).await.unwrap();let mut s=snapshot();s.collations.insert("postgres".into(),inspect::auto_collations(&c.client).await.unwrap());validate(&s).unwrap();assert_eq!(serde_json::to_vec(&before).unwrap(),serde_json::to_vec(&inspect::snapshot_database(&c.client,None,false).await.unwrap()).unwrap());
        c.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();
        for setup in ["BEGIN; CREATE COLLATION public.g2_en(provider=icu,locale='en-US')","BEGIN; CREATE COLLATION public.g2_en(provider=libc,locale='C.UTF-8')"]{
        for ddl in ["CREATE TABLE public.g2_table(label text COLLATE public.g2_en)","CREATE TABLE public.g2_partition(label text) PARTITION BY RANGE(label COLLATE public.g2_en)","CREATE DOMAIN public.g2_domain AS text COLLATE public.g2_en"]{
            c.client.batch_execute(setup).await.unwrap();c.client.batch_execute(ddl).await.unwrap();
            let rows=inspect::auto_collations(&c.client).await.unwrap();assert!(rows.iter().any(|r|r.schema=="public"&&r.name=="g2_en"),"{ddl}");s.collations.insert("postgres".into(),rows);assert_eq!(validate(&s).unwrap_err().code,"collation_unsupported");
            c.client.batch_execute("ROLLBACK").await.unwrap();
        }
        }
        c.client.batch_execute("BEGIN; CREATE COLLATION public.g2_ctype(provider=libc,lc_collate='C',lc_ctype='C.UTF-8'); CREATE TABLE public.g2_mixed(label text COLLATE public.g2_ctype)").await.unwrap();
        let mixed=inspect::auto_collations(&c.client).await.unwrap();assert!(mixed.iter().any(|r|r.name=="g2_ctype"&&r.locale.as_deref()==Some("C.UTF-8")));s.collations.insert("postgres".into(),mixed);assert!(validate(&s).is_err());c.client.batch_execute("ROLLBACK").await.unwrap();
        drop(c);pg.stop().unwrap();
    }
}
