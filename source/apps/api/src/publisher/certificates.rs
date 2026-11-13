//! Issued material is read from one caller-owned REPEATABLE READ transaction.
use crate::crypto::SecretBox;
use super::files::CertificateMaterial;
use super::store::uuid;

const MAX_PEM:usize=1024*1024;

#[derive(Debug,thiserror::Error)]
pub(super) enum Error {
    #[error("stale_material")]
    StaleMaterial,
    #[error("인증서·개인키 자료를 확인할 수 없습니다.")]
    Invalid,
    #[error("인증서 저장소를 읽을 수 없습니다.")]
    Database(#[from] tokio_postgres::Error),
    #[error("인증서 자료 검증을 완료하지 못했습니다.")]
    Worker,
}

// No Debug or Serialize: the private key stays inside the publisher.
pub(super) struct Validated {
    material:CertificateMaterial,
    digest:String,
}
impl Validated {
    pub fn id(&self) -> &str {&self.material.id}
    pub fn content_digest(&self) -> &str {&self.digest}
    pub fn into_material(self) -> CertificateMaterial {self.material}
}

pub(super) use crate::control::model::certificate_content_digest as content_digest;

fn open(id:&str,cert:String,sealed:String,secrets:&SecretBox,expected:Option<&str>) -> Result<Validated,Error> {
    uuid(id).map_err(|_|Error::Invalid)?;
    let digest=content_digest(&cert,&sealed);
    if expected.is_some_and(|expected|expected!=digest) {return Err(Error::StaleMaterial);}
    if cert.len()>MAX_PEM || sealed.len()>MAX_PEM {return Err(Error::Invalid);}
    let key=secrets.open_maybe_plain(Some(&sealed)).ok_or(Error::Invalid)?;
    if key.len()>MAX_PEM {return Err(Error::Invalid);}
    Ok(Validated {material:CertificateMaterial {id:id.into(),cert_pem:cert,key_pem:key},digest})
}

fn leaf_key(value:Validated) -> Result<Validated,Error> {
    crate::certificates::pem::parse_certificate_pem(&value.material.cert_pem,&value.material.key_pem).map_err(|_|Error::Invalid)?;
    Ok(value)
}

pub(super) async fn read(tx:&deadpool_postgres::Transaction<'_>,id:&str,secrets:&SecretBox,expected:Option<&str>) -> Result<Validated,Error> {
    uuid(id).map_err(|_|Error::Invalid)?;
    let row=tx.query_opt("SELECT id,cert_pem,key_pem FROM certificates WHERE id=$1 AND status='issued'",&[&id]).await?.ok_or(Error::Invalid)?;
    let cert:Option<String>=row.get(1);
    let sealed:Option<String>=row.get(2);
    let value=open(id,cert.ok_or(Error::Invalid)?,sealed.ok_or(Error::Invalid)?,secrets,expected)?;
    tokio::task::spawn_blocking(move||leaf_key(value)).await.map_err(|_|Error::Worker)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n1_certificates_accept_issued_self_signed_and_legacy_plaintext_keys() {
        let secrets=SecretBox::from_key([11;32]);
        let pair=rcgen::generate_simple_self_signed(vec!["example.test".into()]).unwrap();
        let cert=pair.cert.pem();let key=pair.signing_key.serialize_pem();
        let id=uuid::Uuid::new_v4().to_string();
        for stored in [key.clone(),secrets.seal(&key).unwrap()] {
            let expected=content_digest(&cert,&stored);
            let value=leaf_key(open(&id,cert.clone(),stored,&secrets,Some(&expected)).unwrap()).unwrap();
            assert_eq!(value.id(),id);assert_eq!(value.content_digest(),expected);
            assert_eq!(value.into_material().key_pem,key);
        }
    }

    #[test]
    fn n1_certificates_refuse_stale_mismatch_malformed_sealed_values_and_oversized_pem() {
        let secrets=SecretBox::from_key([11;32]);
        let pair=rcgen::generate_simple_self_signed(vec!["example.test".into()]).unwrap();
        let other=rcgen::generate_simple_self_signed(vec!["example.test".into()]).unwrap();
        let cert=pair.cert.pem();let key=pair.signing_key.serialize_pem();
        let id=uuid::Uuid::new_v4().to_string();
        assert!(matches!(open(&id,cert.clone(),key.clone(),&secrets,Some(&"a".repeat(64))),Err(Error::StaleMaterial)));
        assert!(open("../outside",cert.clone(),key.clone(),&secrets,None).is_err());
        assert!(open(&id,cert.clone(),"enc:v1:broken".into(),&secrets,None).is_err());
        assert!(open(&id,"x".repeat(MAX_PEM+1),key.clone(),&secrets,None).is_err());
        assert!(leaf_key(open(&id,"malformed".into(),key,&secrets,None).unwrap()).is_err());
        assert!(leaf_key(open(&id,cert,other.signing_key.serialize_pem(),&secrets,None).unwrap()).is_err());
    }
}
