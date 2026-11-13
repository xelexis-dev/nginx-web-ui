use super::{state::{self,Destination,Destinations},error::{Error,Result}};
#[derive(Debug,PartialEq,Eq,Clone,Copy)]
pub enum Operation { Check,Run,Status,Verify }
#[derive(Debug)]
pub struct Arguments {
    pub operation:Operation,pub json:bool,pub backup_id:Option<String>,pub destinations:Option<Destinations>,
}
impl Arguments {
    pub fn parse(args:impl IntoIterator<Item=String>)->Result<Self> {
        let bad=||Error::refusal("invalid_arguments");
        let mut args=args.into_iter();
        let operation=match args.next().as_deref(){Some("check")=>Operation::Check,Some("run")=>Operation::Run,Some("status")=>Operation::Status,Some("verify")=>Operation::Verify,_=>return Err(bad())};
        let mut json=false;let mut backup_id=None;let mut db=None;let mut app=None;
        while let Some(flag)=args.next() {
            if flag=="--json" {if json{return Err(bad());}json=true;continue;}
            if !matches!(operation,Operation::Check|Operation::Run){return Err(bad());}
            let value=args.next().ok_or_else(bad)?;
            match flag.as_str() {
                "--backup-id" if backup_id.is_none() && state::backup_id(&value)=>backup_id=Some(value),
                "--destination-db-volume" if db.is_none()=>db=Some(Destination::Volume{source:value}),
                "--destination-db-bind" if db.is_none()=>db=Some(Destination::Bind{source:value}),
                "--destination-app-volume" if app.is_none()=>app=Some(Destination::Volume{source:value}),
                "--destination-app-bind" if app.is_none()=>app=Some(Destination::Bind{source:value}),
                _=>return Err(bad()),
            }
        }
        let destinations=if matches!(operation,Operation::Check|Operation::Run) {
            let db=db.ok_or_else(bad)?;let app=app.ok_or_else(bad)?;db.validate().map_err(|_|bad())?;app.validate().map_err(|_|bad())?;
            if backup_id.is_none(){return Err(bad());}
            match (&db,&app) {
                (Destination::Volume{source:a},Destination::Volume{source:b}) if a==b=>return Err(bad()),
                (Destination::Bind{source:a},Destination::Bind{source:b}) if std::path::Path::new(a).starts_with(b)||std::path::Path::new(b).starts_with(a)=>return Err(bad()),
                _=>{},
            }
            Some(Destinations{db,app})
        }else{None};
        Ok(Self{operation,json,backup_id,destinations})
    }
}
