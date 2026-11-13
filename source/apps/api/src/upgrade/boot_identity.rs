//! A.19 container start identity shared by the runner and privilege receipt.
use super::StepError;
use sha2::{Digest,Sha256};
use std::{fs::OpenOptions,io::Read,os::unix::fs::OpenOptionsExt,path::Path};
const MAX_INPUT:u64=65536;
fn unavailable()->StepError {StepError {code:"boot_step_failed",message:"컨테이너 시작 정보를 확인하지 못했습니다. 이전 실행 상태를 재사용하지 않습니다.",detail:Some("boot_identity".into())}}
fn read_input(path:&Path)->Result<String,StepError> {
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path).map_err(|_|unavailable())?;
    let mut text=String::new();file.take(MAX_INPUT+1).read_to_string(&mut text).map_err(|_|unavailable())?;
    if text.len()as u64>MAX_INPUT {return Err(unavailable());}Ok(text)
}
fn identity(boot_id:&str,stat:&str)->Result<String,StepError> {
    if boot_id.len()as u64>MAX_INPUT || stat.len()as u64>MAX_INPUT || stat.contains('\0') {return Err(unavailable());}
    let boot_id=boot_id.trim();let id=uuid::Uuid::parse_str(boot_id).map_err(|_|unavailable())?;
    if id.hyphenated().to_string()!=boot_id {return Err(unavailable());}
    let open=stat.find('(').ok_or_else(unavailable)?;let close=stat.rfind(')').ok_or_else(unavailable)?;
    if open>=close || stat[..open].trim()!="1" {return Err(unavailable());}
    // The suffix starts at field 3; comm can contain spaces and both parentheses.
    let start=stat[close+1..].split_whitespace().nth(19).ok_or_else(unavailable)?;
    if start.is_empty() || !start.bytes().all(|b|b.is_ascii_digit()) {return Err(unavailable());}
    let start=start.parse::<u64>().map_err(|_|unavailable())?.to_string();
    let mut hash=Sha256::new();hash.update(b"nwui-boot-v1\0");hash.update(boot_id.as_bytes());hash.update(b"\0");hash.update(start.as_bytes());
    Ok(hex::encode(hash.finalize()))
}
fn from_files(boot:&Path,stat:&Path)->Result<String,StepError> {identity(&read_input(boot)?,&read_input(stat)?)}
/// Err means phase 1 for the runner; N2 maps it to privilege_prepare_failed and writes no receipt.
/// No namespace inode, environment input, AUTH_SECRET, or fallback identity is used.
pub fn current()->Result<String,StepError> {
    from_files(Path::new("/proc/sys/kernel/random/boot_id"),Path::new("/proc/1/stat"))
}
#[cfg(test)]
mod tests {
    use super::*;
    const BOOT:&str="3d521ab4-34ad-44e8-b7eb-882c0feff693";
    fn stat(comm:&str,start:&str)->String {
        let mut fields=vec!["S".to_owned()];fields.extend((0..18).map(|_|"0".to_owned()));fields.push(start.into());fields.extend((0..8).map(|_|"0".to_owned()));format!("1 ({comm}) {}\n",fields.join(" "))
    }
    #[test]
    fn boot_identity_matches_the_frozen_domain_separated_hash_with_nested_comm() {
        let got=identity(BOOT,&stat("s6 ) ( init","12345")).unwrap();
        assert_eq!(got,"ad997f7a8260d879cb599f1dfd754f9efb92cd9a54756d8f309b3afb67faa860");assert_eq!(got,identity(&format!("{BOOT}\n"),&stat("different comm","12345")).unwrap());
    }
    #[test]
    fn boot_identity_stays_fixed_for_runner_restart_but_changes_for_pid1_or_kernel_restart() {
        let a=identity(BOOT,&stat("init","12345")).unwrap();assert_eq!(a,identity(BOOT,&stat("init","12345")).unwrap());assert_ne!(a,identity(BOOT,&stat("init","12346")).unwrap());
        assert_ne!(a,identity("f9897461-085b-43af-8534-d191a0082c03",&stat("init","12345")).unwrap());
        identity(BOOT,&stat("init","0")).unwrap();
    }
    #[test]
    fn boot_identity_refuses_malformed_or_unreadable_inputs_without_a_fallback() {
        for boot in ["","not-a-uuid","3D521AB4-34AD-44E8-B7EB-882C0FEFF693","urn:uuid:3d521ab4-34ad-44e8-b7eb-882c0feff693"] {assert!(identity(boot,&stat("init","12345")).is_err());}
        for bad in ["1 init 0".into(),"1 (init) S 0".into(),stat("init","-1"),stat("init","abc"),stat("init","18446744073709551616"),stat("init","12345").replacen("1 (","2 (",1)] {assert!(identity(BOOT,&bad).is_err());}
        let t=tempfile::tempdir().unwrap();let boot=t.path().join("boot");std::fs::write(&boot,BOOT).unwrap();assert!(from_files(&boot,&t.path().join("absent-stat")).is_err());
    }
    #[test]
    fn boot_identity_bounds_fault_inputs_and_accepts_a_u64_starttime() {
        assert!(identity(BOOT,&"x".repeat(MAX_INPUT as usize+1)).is_err());identity(BOOT,&stat("init","18446744073709551615")).unwrap();
    }
    #[test]
    fn boot_identity_reads_only_the_current_kernel_and_pid1_inputs() {
        let first=current().unwrap();assert_eq!(first.len(),64);assert!(first.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));assert_eq!(first,current().unwrap());
    }
}
