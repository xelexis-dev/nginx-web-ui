//! Drop in the process that execs the API; never exec a root UID helper afterwards.
use std::io;

pub const API_UID: u32 = 10001;
pub const API_GID: u32 = 10001;

#[derive(Clone, Debug)]
pub(super) struct Identity { pub groups: Vec<u32>, last_cap: u32, bounding: u64, can_drop_bounding: bool }

impl Identity {
    pub fn read() -> io::Result<Self> {
        if unsafe { libc::geteuid() } != 0 { return Err(invalid("root pre-exec required")); }
        let group_file = std::fs::read_to_string("/etc/group")?;
        let groups = ["nginx", "adm"].into_iter().map(|name| {
            group_file.lines().find_map(|line| {
                let fields = line.split(':').collect::<Vec<_>>();
                (fields.len() == 4 && fields[0] == name).then(|| fields[2].parse::<u32>().ok()).flatten()
            }).ok_or_else(|| invalid("missing API supplementary group"))
        }).collect::<io::Result<Vec<_>>>()?;
        Self::for_groups(groups)
    }

    #[cfg(test)]
    pub(super) fn for_test_groups(groups: Vec<u32>) -> Self { Self::for_groups(groups).unwrap() }

    fn for_groups(groups: Vec<u32>) -> io::Result<Self> {
        let status = std::fs::read_to_string("/proc/thread-self/status")?;
        let effective = hex_field(&status, "CapEff")?;
        Ok(Self {
            groups,
            last_cap: std::fs::read_to_string("/proc/sys/kernel/cap_last_cap")?.trim().parse().map_err(|_| invalid("invalid capability limit"))?,
            bounding: hex_field(&status, "CapBnd")?,
            can_drop_bounding: effective & (1 << 8) != 0,
        })
    }

    pub fn drop(&self, uid: u32, gid: u32) -> io::Result<()> {
        if self.can_drop_bounding {
            for capability in 0..=self.last_cap {
                syscall(unsafe { libc::prctl(libc::PR_CAPBSET_DROP, capability, 0, 0, 0) })?;
            }
        } else {
            let message = b"[nonroot] bounding_set_retained\n";
            unsafe { libc::write(2, message.as_ptr().cast(), message.len()); }
        }
        syscall(unsafe { libc::setgroups(self.groups.len(), self.groups.as_ptr()) })?;
        syscall(unsafe { libc::setresgid(gid, gid, gid) })?;
        syscall(unsafe { libc::setresuid(uid, uid, uid) })?;
        #[repr(C)] struct Header { version: u32, pid: i32 }
        #[repr(C)] #[derive(Clone, Copy)] struct Data { effective: u32, permitted: u32, inheritable: u32 }
        let header = Header { version: 0x20080522, pid: 0 };
        let data = [Data { effective: 0, permitted: 0, inheritable: 0 }; 2];
        syscall(unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) } as i32)?;
        syscall(unsafe { libc::prctl(libc::PR_CAP_AMBIENT, libc::PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0) })?;
        syscall(unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) })?;
        // The probe also uses this after fork: no allocator or Rust I/O locks on success.
        let fd = unsafe { libc::open(c"/proc/thread-self/status".as_ptr(), libc::O_RDONLY|libc::O_CLOEXEC) };
        if fd == -1 { return Err(io::Error::last_os_error()); }
        let mut bytes = [0u8; 8192];
        let count = unsafe { libc::read(fd, bytes.as_mut_ptr().cast(), bytes.len()) };
        let error = io::Error::last_os_error();
        unsafe { libc::close(fd); }
        if count < 0 { return Err(error); }
        if count as usize == bytes.len() { return Err(invalid("oversized process status")); }
        let status = std::str::from_utf8(&bytes[..count as usize]).map_err(|_| invalid("invalid process status"))?;
        verify(status, uid, gid, &self.groups,
            if self.can_drop_bounding { 0 } else { self.bounding })
    }
}

fn invalid(_message: &'static str) -> io::Error { io::Error::from_raw_os_error(libc::EPERM) }
fn syscall(result: i32) -> io::Result<()> { if result == -1 { Err(io::Error::last_os_error()) } else { Ok(()) } }
fn field<'a>(status: &'a str, name: &str) -> io::Result<&'a str> {
    status.lines().find_map(|line| line.strip_prefix(name).and_then(|s| s.strip_prefix(':')))
        .map(str::trim).ok_or_else(|| invalid("missing process identity field"))
}
fn hex_field(status: &str, name: &str) -> io::Result<u64> {
    u64::from_str_radix(field(status, name)?, 16).map_err(|_| invalid("invalid process capability field"))
}
fn verify(status: &str, uid: u32, gid: u32, groups: &[u32], bounding: u64) -> io::Result<()> {
    for (name, wanted) in [("Uid", uid), ("Gid", gid)] {
        let mut ids = field(status, name)?.split_whitespace();
        for _ in 0..4 {
            if ids.next().and_then(|id| id.parse::<u32>().ok()) != Some(wanted) { return Err(invalid("API identity was not dropped")); }
        }
        if ids.next().is_some() { return Err(invalid("unexpected process identity field")); }
    }
    let mut count = 0;
    for group in field(status, "Groups")?.split_whitespace() {
        let group = group.parse::<u32>().map_err(|_| invalid("invalid supplementary groups"))?;
        if !groups.contains(&group) { return Err(invalid("unexpected supplementary group")); }
        count += 1;
    }
    if count != groups.len() { return Err(invalid("unexpected supplementary groups")); }
    for name in ["CapInh", "CapPrm", "CapEff", "CapAmb"] {
        if hex_field(status, name)? != 0 { return Err(invalid("API retained capabilities")); }
    }
    if hex_field(status, "CapBnd")? != bounding || field(status, "NoNewPrivs")? != "1" {
        return Err(invalid("API privilege boundary was not established"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const STATUS: &str = "Uid:\t10001 10001 10001 10001\nGid:\t10001 10001 10001 10001\nGroups:\t4 101\nCapInh:\t0000000000000000\nCapPrm:\t0000000000000000\nCapEff:\t0000000000000000\nCapBnd:\t0000000000000000\nCapAmb:\t0000000000000000\nNoNewPrivs:\t1\n";
    #[test]
    fn identity_verification_checks_saved_uid_groups_and_every_capability() {
        verify(STATUS, API_UID, API_GID, &[101,4], 0).unwrap();
        for bad in [STATUS.replace("10001 10001 10001 10001", "10001 10001 0 10001"), STATUS.replace("4 101", "0 4 101"), STATUS.replace("NoNewPrivs:\t1", "NoNewPrivs:\t0")] {
            assert!(verify(&bad, API_UID, API_GID, &[101,4], 0).is_err());
        }
        for name in ["CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"] {
            assert!(verify(&STATUS.replace(&format!("{name}:\t0000000000000000"), &format!("{name}:\t0000000000000001")), API_UID, API_GID, &[101,4], 0).is_err());
        }
        let retained = STATUS.replace("CapBnd:\t0000000000000000", "CapBnd:\t00000000a80425fb");
        verify(&retained, API_UID, API_GID, &[101,4], 0xa80425fb).unwrap();
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use std::{os::unix::process::CommandExt, process::Command};

    #[test]
    fn real_drop_clears_inheritable_saved_ids_and_capabilities_before_exec() {
        let identity = Identity::for_groups(vec![4, 101]).unwrap();
        let mut child = Command::new("/bin/cat");
        child.arg("/proc/thread-self/status");
        unsafe { child.pre_exec(move || {
            #[repr(C)] struct Header { version: u32, pid: i32 }
            #[repr(C)] #[derive(Clone, Copy)] struct Data { effective: u32, permitted: u32, inheritable: u32 }
            let header = Header { version: 0x20080522, pid: 0 };
            let mut data = [Data { effective: 0, permitted: 0, inheritable: 0 };2];
            syscall(libc::syscall(libc::SYS_capget, &header, data.as_mut_ptr()) as i32)?;
            data[0].inheritable = data[0].permitted & 1;
            syscall(libc::syscall(libc::SYS_capset, &header, data.as_ptr()) as i32)?;
            identity.drop(API_UID, API_GID)
        }); }
        let result = child.output().unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        let status = String::from_utf8(result.stdout).unwrap();
        let initial = std::fs::read_to_string("/proc/thread-self/status").unwrap();
        let expected = if hex_field(&initial,"CapEff").unwrap() & (1<<8) == 0 { hex_field(&initial,"CapBnd").unwrap() } else { 0 };
        verify(&status, API_UID, API_GID, &[4,101], expected).unwrap();
        if expected != 0 { assert!(String::from_utf8_lossy(&result.stderr).contains("bounding_set_retained")); }

    }
}
