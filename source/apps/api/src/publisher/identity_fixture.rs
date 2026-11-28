//! Process and supervisor fixtures. No override is compiled into release builds.
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{LazyLock, Mutex},
};

#[derive(Clone)]
pub(super) struct Override {
    pub group_file: PathBuf,
    pub executable: PathBuf,
    pub pid: Option<u32>,
}

static OVERRIDES: LazyLock<Mutex<HashMap<PathBuf, Override>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn identity_override(pid_file: &Path) -> Option<Override> {
    OVERRIDES.lock().unwrap().get(pid_file).cloned()
}

pub struct RegisteredMaster {
    pid_file: PathBuf,
    pid: Option<u32>,
}

impl RegisteredMaster {
    pub fn new(pid_file: &Path, executable: &Path, pid: u32) -> Self {
        Self::register(pid_file, executable, Some(pid))
    }

    pub fn stopped(pid_file: &Path, executable: &Path) -> Self {
        Self::register(pid_file, executable, None)
    }

    fn register(pid_file: &Path, executable: &Path, pid: Option<u32>) -> Self {
        let mut overrides = OVERRIDES.lock().unwrap();
        let group_file = pid_file.with_extension("group");
        if let Some(pid) = pid {
            std::fs::write(&group_file, pid.to_string()).unwrap();
        } else {
            assert!(!group_file.exists());
        }
        let fixture = Override { group_file, executable: executable.into(), pid };
        overrides.insert(pid_file.into(), fixture);
        Self { pid_file: pid_file.into(), pid }
    }
}

impl Drop for RegisteredMaster {
    fn drop(&mut self) {
        let mut overrides = OVERRIDES.lock().unwrap();
        // A later witness can replace the master on the same fixture path.
        if overrides.get(&self.pid_file).is_some_and(|current| current.pid == self.pid) {
            let current = overrides.remove(&self.pid_file).unwrap();
            let _ = std::fs::remove_file(current.group_file);
        }
    }
}

pub struct FakeMaster {
    child: Child,
    _registered: RegisteredMaster,
}

impl FakeMaster {
    pub fn spawn(pid_file: &Path, catch_hup: bool) -> Self {
        let script = if catch_hup {
            "trap ':' HUP; printf 'ready\\n'; while read -r line; do :; done"
        } else {
            "printf 'ready\\n'; while read -r line; do :; done"
        };
        let mut child = Command::new("/bin/bash").args(["-c", script]).env_clear()
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        let pid = child.id();
        let master = Self { child, _registered: RegisteredMaster::new(pid_file, Path::new("/bin/bash"), pid) };
        assert_eq!(line, "ready\n");
        std::fs::write(pid_file, pid.to_string()).unwrap();
        master
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for FakeMaster {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn superseded_witness_leaves_the_replacement_registered() {
        let root = tempfile::tempdir().unwrap();
        let pid_file = root.path().join("nginx.pid");
        let first = FakeMaster::spawn(&pid_file, true);
        let replacement = FakeMaster::spawn(&pid_file, true);
        let expected = replacement.pid();
        drop(first);

        let current = identity_override(&pid_file).unwrap();
        assert_eq!(current.pid, Some(expected));
        assert_eq!(std::fs::read_to_string(&current.group_file).unwrap(), expected.to_string());
        assert_eq!(std::fs::read_to_string(&pid_file).unwrap(), expected.to_string());
        drop(replacement);
        assert!(identity_override(&pid_file).is_none());
        assert!(!current.group_file.exists());
    }
}
