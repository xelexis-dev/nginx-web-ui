//! A.16 same-name entry moves through already-authorised directory capabilities.
use super::{Journal, MovePhase, MoveRecord, StepError, StepState, StorageLocks};
use std::{ffi::CString, fs::File, io, os::{fd::AsRawFd, unix::fs::MetadataExt}};

pub struct MoveDirs<'a> {
    pub l: &'a File,
    pub bk: &'a File,
    pub w_linked: &'a File,
}

fn mismatch() -> StepError {
    StepError { code: "pair_mismatch", message: "항목 이동 기록과 저장소 배치가 다릅니다. 기존 내용을 보존했습니다.", detail: None }
}
fn placement_mismatch(record: &MoveRecord, placement: (bool, bool)) -> StepError {
    let (phase, from, to) = match record.phase {
        MovePhase::SetAside => ("set-aside", "L", "W-linked"),
        MovePhase::Restore => ("restore", "BK", "L"),
        _ => return mismatch(),
    };
    let present = |value| if value { "present" } else { "absent" };
    // The validated name is at most 255 bytes; Debug escapes controls within the 2 KiB detail budget.
    let detail = format!("{phase} {:?}: {from}->{to}, placement src={} dst={}",
        record.entry, present(placement.0), present(placement.1));
    debug_assert!(detail.len() <= 2048);
    StepError { detail: Some(detail), ..mismatch() }
}
fn io_error(error: io::Error) -> StepError {
    let code = match error.raw_os_error() {
        Some(libc::EROFS) => "storage_read_only",
        Some(libc::ENOSPC | libc::EDQUOT) => "insufficient_space",
        Some(libc::EINTR) => "interrupted",
        _ => "io_error",
    };
    StepError { code, message: "항목 이동을 완료하지 못했습니다. 복구 기록과 기존 내용을 보존했습니다.", detail: None }
}
fn safe_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 255 && !name.contains(['/', '\0'])
        && !matches!(name, "." | ".." | "lost+found" | "NWUI-RESTORE-IN-PROGRESS.txt"
            | "UPGRADED-TO-PG17.txt" | ".nwui")
        && !name.starts_with(".nwui-")
}
fn leaf(parent: &File, name: &CString, device: u64) -> Result<bool, StepError> {
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstatat(parent.as_raw_fd(), name.as_ptr(), metadata.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::NotFound { Ok(false) } else { Err(io_error(error)) };
    }
    let metadata = unsafe { metadata.assume_init() };
    if metadata.st_dev != device || !matches!(metadata.st_mode & libc::S_IFMT, libc::S_IFREG | libc::S_IFDIR) {
        return Err(mismatch());
    }
    Ok(true)
}
#[cfg(test)]
fn checkpoint(point: &str) {
    if std::env::var("NWUI_MOVE_KILL_POINT").as_deref() == Ok(point) {
        unsafe { libc::kill(libc::getpid(), libc::SIGKILL); }
    }
}
#[cfg(not(test))]
fn checkpoint(_point: &str) {}

pub fn move_entry(
    locks: &StorageLocks,
    journal: &mut Journal,
    dirs: &MoveDirs<'_>,
    allowed: &dyn Fn(&str) -> bool,
    record: &MoveRecord,
) -> Result<(), StepError> {
    let (source, destination) = match (record.phase, record.from.as_str(), record.to.as_str()) {
        (MovePhase::SetAside, "L", "W") => (dirs.l, dirs.w_linked),
        (MovePhase::Restore, "BK", "L") => (dirs.bk, dirs.l),
        _ => return Err(mismatch()),
    };
    if !safe_name(&record.entry) || !allowed(&record.entry) { return Err(mismatch()); }
    if !locks.holds_a() || !locks.holds_d() {
        return Err(StepError { code: "storage_busy", message: "저장소 잠금이 유지되지 않았습니다.", detail: None });
    }
    let previous = journal.load().moves.iter().rev()
        .find(|m| m.record.phase == record.phase && m.record.entry == record.entry);
    if let Some(previous) = previous {
        if previous.record.from != record.from || previous.record.to != record.to || previous.record.state != record.state {
            return Err(mismatch());
        }
    } else if record.state != StepState::Intent { return Err(mismatch()); }
    if matches!(record.state, StepState::Failed { .. }) { return Err(mismatch()); }
    let new = previous.is_none();
    let metadata = [dirs.l, dirs.bk, dirs.w_linked].map(|dir| dir.metadata().map_err(io_error));
    let [l, bk, w] = metadata;
    let (l, bk, w) = (l?, bk?, w?);
    if !l.is_dir() || !bk.is_dir() || !w.is_dir() || l.dev() != bk.dev() || l.dev() != w.dev()
        || l.ino() == bk.ino() || l.ino() == w.ino() || bk.ino() == w.ino() { return Err(mismatch()); }
    let name = CString::new(record.entry.as_str()).map_err(|_| mismatch())?;
    let placement = (leaf(source, &name, l.dev())?, leaf(destination, &name, l.dev())?);
    if record.state == StepState::Done {
        return if placement == (false, true) { Ok(()) } else { Err(placement_mismatch(record, placement)) };
    }
    if new && placement != (true, false) || !new && !matches!(placement, (true, false) | (false, true)) {
        return Err(placement_mismatch(record, placement));
    }
    if new { journal.record_move(record.clone())?; checkpoint("intent"); }
    if placement.0 {
        // Both storage locks exclude another authorised writer; never replace an existing leaf.
        if leaf(destination, &name, l.dev())? { return Err(placement_mismatch(record, (leaf(source, &name, l.dev())?, true))); }
        if unsafe { libc::renameat(source.as_raw_fd(), name.as_ptr(), destination.as_raw_fd(), name.as_ptr()) } != 0 {
            let error = io::Error::last_os_error();
            return Err(if matches!(error.raw_os_error(), Some(libc::EXDEV | libc::EEXIST | libc::ENOTEMPTY)) { mismatch() } else { io_error(error) });
        }
        checkpoint("rename");
    }
    source.sync_all().map_err(io_error)?; checkpoint("source-fsync");
    destination.sync_all().map_err(io_error)?; checkpoint("destination-fsync");
    let placement = (leaf(source, &name, l.dev())?, leaf(destination, &name, l.dev())?);
    if placement != (false, true) { return Err(placement_mismatch(record, placement)); }
    let mut done = record.clone(); done.state = StepState::Done;
    journal.record_move(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upgrade::{Branch, MovePhase, StepState, journal::Candidate};
    use std::{fs::{self, OpenOptions}, os::unix::{fs::{MetadataExt, OpenOptionsExt, PermissionsExt, symlink}, process::ExitStatusExt}, path::{Path, PathBuf}};

    struct Fixture { _tmp: tempfile::TempDir, db: PathBuf, app: PathBuf }
    impl Fixture {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let db = tmp.path().join("db"); let app = tmp.path().join("app");
            for p in [&db, &app] {
                fs::create_dir(p).unwrap(); fs::set_permissions(p, fs::Permissions::from_mode(0o700)).unwrap();
            }
            for p in [db.join("l"), db.join("bk"), db.join("linked")] { fs::create_dir(p).unwrap(); }
            let locks = StorageLocks::acquire(&db, &app).unwrap();
            {let fixture_journal=Journal::begin(&app, &locks, Branch::Upgrade, Candidate {
                revision: "1".repeat(40), pins_sha256: "2".repeat(64), migrations_sha256: "3".repeat(64),
            }).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
            Self { _tmp: tmp, db, app }
        }
        fn open(&self) -> (StorageLocks, Journal) {
            let locks = StorageLocks::acquire(&self.db, &self.app).unwrap();
            let journal = Journal::open(&self.app, &locks).unwrap(); (locks, journal)
        }
        fn caps(&self) -> [File; 3] {
            ["l", "bk", "linked"].map(|n| OpenOptions::new().read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW).open(self.db.join(n)).unwrap())
        }
        fn path(&self, alias: &str) -> PathBuf {
            self.db.join(match alias { "L" => "l", "BK" => "bk", "W" => "linked", _ => panic!("alias") }).join("base")
        }
    }
    fn dirs(c: &[File; 3]) -> MoveDirs<'_> { MoveDirs { l: &c[0], bk: &c[1], w_linked: &c[2] } }
    fn record(phase: MovePhase, state: StepState) -> MoveRecord {
        let (from, to) = match phase { MovePhase::Restore => ("BK", "L"), _ => ("L", "W") };
        MoveRecord { phase, entry: "base".into(), from: from.into(), to: to.into(), state }
    }
    fn assert_payload(path: &Path, ino: u64) {
        assert_eq!(fs::read(path.join("payload")).unwrap(), b"preserve bytes");
        assert_eq!(fs::metadata(path).unwrap().ino(), ino);
    }
    fn seed(path: &Path) -> u64 {
        fs::create_dir(path).unwrap(); fs::write(path.join("payload"), b"preserve bytes").unwrap();
        fs::metadata(path).unwrap().ino()
    }
    #[test]
    fn both_legal_moves_are_durable_idempotent_and_preserve_identity() {
        for phase in [MovePhase::SetAside, MovePhase::Restore] {
            let f = Fixture::new(); let c = f.caps(); let (locks, mut j) = f.open();
            let r = record(phase, StepState::Intent); let ino = seed(&f.path(&r.from));
            move_entry(&locks, &mut j, &dirs(&c), &|n| n == "base", &r).unwrap();
            assert!(!f.path(&r.from).exists()); assert_payload(&f.path(&r.to), ino);
            assert_eq!(j.load().moves.len(), 2);
            assert_eq!(j.load().moves[0].record.state, StepState::Intent);
            let done = j.load().moves.last().unwrap().record.clone();
            assert_eq!(done.state, StepState::Done);
            move_entry(&locks, &mut j, &dirs(&c), &|_| true, &done).unwrap();
            assert_eq!(j.load().moves.len(), 2);
            let mut reopened = Journal::open(&f.app, &locks).unwrap();
            move_entry(&locks, &mut reopened, &dirs(&c), &|_| true, &done).unwrap();
        }
    }
    #[test]
    fn resume_checks_every_intent_and_done_placement_and_never_adopts_a_new_destination() {
        for phase in [MovePhase::SetAside, MovePhase::Restore] {
            for state in [StepState::Intent, StepState::Done, StepState::Failed { code: "io_error".into() }] {
                for source in [false, true] { for destination in [false, true] {
                    let f = Fixture::new(); let c = f.caps(); let (locks, mut j) = f.open();
                    let r = record(phase, state.clone()); j.record_move(r.clone()).unwrap();
                    if source { seed(&f.path(&r.from)); } if destination { seed(&f.path(&r.to)); }
                    let before = j.load().moves.len();
                    let ok = match state { StepState::Intent => source != destination, StepState::Done => !source && destination, _ => false };
                    let result = move_entry(&locks, &mut j, &dirs(&c), &|_| true, &r);
                    assert_eq!(result.is_ok(), ok, "{phase:?} {state:?} {source}/{destination}");
                    if !ok {
                        assert_eq!(result.unwrap_err().code, "pair_mismatch");
                        assert_eq!(f.path(&r.from).exists(), source); assert_eq!(f.path(&r.to).exists(), destination);
                        assert_eq!(j.load().moves.len(), before);
                    }
                }}
            }
            let f = Fixture::new(); let c = f.caps(); let (locks, mut j) = f.open();
            let r = record(phase, StepState::Intent); seed(&f.path(&r.to));
            assert_eq!(move_entry(&locks, &mut j, &dirs(&c), &|_| true, &r).unwrap_err().code, "pair_mismatch");
            assert!(j.load().moves.is_empty()); assert!(f.path(&r.to).exists());
        }
    }
    #[test]
    fn closed_alias_table_and_membership_refuse_before_capability_io() {
        let f = Fixture::new(); let bad = File::open("/dev/null").unwrap(); let (locks, mut j) = f.open();
        let d = MoveDirs { l: &bad, bk: &bad, w_linked: &bad };
        for phase in [MovePhase::SetAside, MovePhase::Restore, MovePhase::Target, MovePhase::Guard, MovePhase::Cleanup] {
            for from in ["L", "BK", "W", "D17", "unknown"] { for to in ["L", "BK", "W", "D17", "unknown"] {
                if matches!((phase, from, to), (MovePhase::SetAside, "L", "W") | (MovePhase::Restore, "BK", "L")) { continue; }
                let mut r = record(phase, StepState::Intent); r.from = from.into(); r.to = to.into();
                assert_eq!(move_entry(&locks, &mut j, &d, &|_| panic!("illegal triple reached membership"), &r).unwrap_err().code, "pair_mismatch");
            }}
        }
        for state in [StepState::Intent, StepState::Done, StepState::Failed { code: "io_error".into() }] {
            let r = record(MovePhase::SetAside, state);
            assert_eq!(move_entry(&locks, &mut j, &d, &|_| false, &r).unwrap_err().code, "pair_mismatch");
        }
        assert!(j.load().moves.is_empty());
    }
    #[test]
    fn membership_is_rechecked_for_new_resumed_and_done_moves() {
        let f = Fixture::new(); let c = f.caps(); let (locks, mut j) = f.open();
        let r = record(MovePhase::SetAside, StepState::Intent); seed(&f.path("L"));
        for persisted in [false, true] {
            if persisted { j.record_move(r.clone()).unwrap(); }
            let n = j.load().moves.len();
            assert_eq!(move_entry(&locks, &mut j, &dirs(&c), &|_| false, &r).unwrap_err().code, "pair_mismatch");
            assert_eq!(j.load().moves.len(), n); assert!(f.path("L").exists());
        }
        move_entry(&locks, &mut j, &dirs(&c), &|_| true, &r).unwrap();
        let done = j.load().moves.last().unwrap().record.clone(); let n = j.load().moves.len();
        assert_eq!(move_entry(&locks, &mut j, &dirs(&c), &|_| false, &done).unwrap_err().code, "pair_mismatch");
        assert_eq!(j.load().moves.len(), n);
    }
    #[test]
    fn unsafe_names_leaves_and_same_inode_parents_are_refused_without_journaling() {
        let f = Fixture::new(); let c = f.caps(); let (locks, mut j) = f.open();
        for name in ["", ".", "..", "a/b", "a\0b", "NWUI-RESTORE-IN-PROGRESS.txt", ".nwui-auto", "lost+found"] {
            let mut r = record(MovePhase::SetAside, StepState::Intent); r.entry = name.into();
            assert_eq!(move_entry(&locks, &mut j, &dirs(&c), &|_| true, &r).unwrap_err().code, "pair_mismatch");
        }
        let outside = f.db.join("outside"); fs::write(&outside, "untouched").unwrap();
        symlink(&outside, f.path("L")).unwrap();
        let r = record(MovePhase::SetAside, StepState::Intent);
        assert_eq!(move_entry(&locks, &mut j, &dirs(&c), &|_| true, &r).unwrap_err().code, "pair_mismatch");
        assert_eq!(fs::read(&outside).unwrap(), b"untouched"); assert!(fs::symlink_metadata(f.path("L")).unwrap().file_type().is_symlink());
        fs::remove_file(f.path("L")).unwrap();
        let fifo = std::ffi::CString::new(f.path("L").as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert_eq!(move_entry(&locks, &mut j, &dirs(&c), &|_| true, &r).unwrap_err().code, "pair_mismatch");
        let same = MoveDirs { l: &c[0], bk: &c[1], w_linked: &c[0] };
        assert_eq!(move_entry(&locks, &mut j, &same, &|_| true, &r).unwrap_err().code, "pair_mismatch");
        assert!(j.load().moves.is_empty());
    }
    #[test]
    fn stale_record_and_new_done_refuse_without_moving() {
        let f = Fixture::new(); let c = f.caps(); let (locks, mut j) = f.open();
        let intent = record(MovePhase::SetAside, StepState::Intent); seed(&f.path("L"));
        let done = record(MovePhase::SetAside, StepState::Done);
        assert_eq!(move_entry(&locks, &mut j, &dirs(&c), &|_| true, &done).unwrap_err().code, "pair_mismatch");
        move_entry(&locks, &mut j, &dirs(&c), &|_| true, &intent).unwrap();
        assert_eq!(move_entry(&locks, &mut j, &dirs(&c), &|_| true, &intent).unwrap_err().code, "pair_mismatch");
        assert_eq!(j.load().moves.len(), 2);
    }
    #[test]
    fn cross_device_capability_is_refused_before_intent() {
        let f = Fixture::new(); let c = f.caps(); let (locks, mut j) = f.open();
        let other = tempfile::tempdir_in("/dev/shm").unwrap(); let foreign = File::open(other.path()).unwrap();
        assert_ne!(foreign.metadata().unwrap().dev(), c[0].metadata().unwrap().dev());
        let d = MoveDirs { l: &c[0], bk: &foreign, w_linked: &c[2] };
        seed(&f.path("L")); let r = record(MovePhase::SetAside, StepState::Intent);
        assert_eq!(move_entry(&locks, &mut j, &d, &|_| true, &r).unwrap_err().code, "pair_mismatch");
        assert!(j.load().moves.is_empty()); assert!(f.path("L").exists());
    }
    #[test]
    fn journal_failure_keeps_intent_and_resumes_after_the_rename() {
        for resumed in [false, true] {
            let f = Fixture::new(); let c = f.caps(); let (locks, mut j) = f.open();
            let r = record(MovePhase::SetAside, StepState::Intent); let ino = seed(&f.path("L"));
            if resumed { j.record_move(r.clone()).unwrap(); }
            let root = f.app.join(".nwui-auto"); let held = f.app.join("held-journal");
            fs::rename(&root, &held).unwrap();
            assert_eq!(move_entry(&locks, &mut j, &dirs(&c), &|_| true, &r).unwrap_err().code, "journal_write_failed");
            assert_eq!(f.path("L").exists(), !resumed);
            assert_eq!(f.path("W").exists(), resumed);
            assert_eq!(j.load().moves.len(), usize::from(resumed));
            if resumed { assert_eq!(j.load().moves[0].record.state, StepState::Intent); }
            fs::rename(&held, &root).unwrap();
            let mut reopened = Journal::open(&f.app, &locks).unwrap();
            move_entry(&locks, &mut reopened, &dirs(&c), &|_| true, &r).unwrap();
            assert_payload(&f.path("W"), ino);
            assert_eq!(reopened.load().moves.last().unwrap().record.state, StepState::Done);
        }
    }
    #[test]
    fn opened_capabilities_move_a_regular_file_even_after_parents_are_renamed() {
        let f = Fixture::new(); let c = f.caps(); let (locks, mut j) = f.open();
        let source = f.path("L"); fs::write(&source, "preserved").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
        let before = fs::metadata(&source).unwrap();
        let moved = f.db.join("renamed-l"); fs::rename(f.db.join("l"), &moved).unwrap();
        let r = record(MovePhase::SetAside, StepState::Intent);
        move_entry(&locks, &mut j, &dirs(&c), &|_| true, &r).unwrap();
        let after = fs::metadata(f.path("W")).unwrap();
        assert_eq!((after.ino(), after.uid(), after.gid(), after.mode()), (before.ino(), before.uid(), before.gid(), before.mode()));
        assert_eq!(fs::read(f.path("W")).unwrap(), b"preserved");
        assert!(!moved.join("base").exists()); assert!(!f.db.join("l").exists());
    }
    #[test]
    fn io_errors_keep_the_frozen_error_codes_and_sanitized_diagnostics() {
        for (errno, code) in [(libc::EROFS, "storage_read_only"), (libc::ENOSPC, "insufficient_space"),
            (libc::EDQUOT, "insufficient_space"), (libc::EINTR, "interrupted"), (libc::EIO, "io_error")] {
            let error = io_error(io::Error::from_raw_os_error(errno));
            assert_eq!(error.code, code); assert!(error.detail.is_none());
        }
    }
    #[test]
    fn frozen_reserved_names_are_exact_and_coordinator_like_names_can_move() {
        for phase in [MovePhase::SetAside, MovePhase::Restore] {
            let f = Fixture::new(); let c = f.caps(); let (locks, mut j) = f.open();
            for name in ["lock", "journal", "journal-v1.json", ".journal-v1.json.init", "postmaster.opts",
                "17", "upgraded-to-pg17.txt", "LOST+FOUND", ".NWUI-data", ".nwui.tmp"] {
                let mut r = record(phase, StepState::Intent); r.entry = name.into();
                let from = f.path(&r.from).with_file_name(name); let to = f.path(&r.to).with_file_name(name);
                fs::write(&from, "preserved").unwrap();
                move_entry(&locks, &mut j, &dirs(&c), &|_| true, &r).unwrap();
                assert!(!from.exists()); assert_eq!(fs::read(to).unwrap(), b"preserved");
            }
        }
        let f = Fixture::new(); let c = f.caps(); let (locks, mut j) = f.open();
        let mut r = record(MovePhase::SetAside, StepState::Intent); r.entry = "17".into();
        fs::write(f.db.join("l/17"), "untouched").unwrap();
        assert_eq!(move_entry(&locks, &mut j, &dirs(&c), &|name| name != "17", &r).unwrap_err().code, "pair_mismatch");
        assert_eq!(fs::read(f.db.join("l/17")).unwrap(), b"untouched");
        assert!(j.load().moves.is_empty());
    }
    #[test]
    fn frozen_reserved_names_are_refused_before_capability_io_in_both_phases() {
        let f = Fixture::new(); let bad = File::open("/dev/null").unwrap(); let (locks, mut j) = f.open();
        let d = MoveDirs { l: &bad, bk: &bad, w_linked: &bad };
        for phase in [MovePhase::SetAside, MovePhase::Restore] {
            for name in ["UPGRADED-TO-PG17.txt", "NWUI-RESTORE-IN-PROGRESS.txt", "lost+found", ".nwui", ".nwui-", ".nwui-nginx-files.lock"] {
                let mut r = record(phase, StepState::Intent); r.entry = name.into();
                assert_eq!(move_entry(&locks, &mut j, &d, &|_| panic!("reserved name reached membership"), &r).unwrap_err().code, "pair_mismatch");
            }
        }
        assert!(j.load().moves.is_empty());
    }
    #[test]
    fn ambiguous_placements_report_exact_relative_aliases_without_changing_data() {
        for phase in [MovePhase::SetAside, MovePhase::Restore] {
            for state in [StepState::Intent, StepState::Done] {
                for present in [false, true] {
                    let f = Fixture::new(); let c = f.caps(); let (locks, mut j) = f.open();
                    let r = record(phase, state.clone()); j.record_move(r.clone()).unwrap();
                    if present { seed(&f.path(&r.from)); seed(&f.path(&r.to)); }
                    let error = move_entry(&locks, &mut j, &dirs(&c), &|_| true, &r).unwrap_err();
                    assert_eq!(error.code, "pair_mismatch");
                    let phase_aliases = if phase == MovePhase::SetAside { "set-aside \"base\": L->W-linked" } else { "restore \"base\": BK->L" };
                    let flag = if present { "present" } else { "absent" };
                    assert_eq!(error.detail.as_deref(), Some(format!("{phase_aliases}, placement src={flag} dst={flag}").as_str()));
                    assert_eq!(f.path(&r.from).exists(), present); assert_eq!(f.path(&r.to).exists(), present);
                    assert_eq!(j.load().moves.len(), 1);
                }
            }
        }
    }
    #[test]
    fn placement_detail_is_bounded_and_escapes_control_characters_in_the_entry() {
        let f = Fixture::new(); let c = f.caps(); let (locks, mut j) = f.open();
        let mut r = record(MovePhase::SetAside, StepState::Intent); r.entry = "\u{1}".repeat(254) + "\n";
        let error = move_entry(&locks, &mut j, &dirs(&c), &|_| true, &r).unwrap_err();
        assert_eq!(error.code, "pair_mismatch"); let detail = error.detail.unwrap();
        assert!(detail.len() <= 2048); assert!(!detail.contains(['\n', '\u{1}']));
        assert!(detail.contains("L->W-linked, placement src=absent dst=absent"));
        assert!(!detail.contains(f._tmp.path().to_str().unwrap())); assert!(j.load().moves.is_empty());
    }
    #[test]
    fn sigkill_child() {
        let Some(base) = std::env::var_os("NWUI_MOVE_FIXTURE") else { return; };
        let base = Path::new(&base); let db = base.join("db"); let app = base.join("app");
        let locks = StorageLocks::acquire(&db, &app).unwrap(); let mut j = Journal::open(&app, &locks).unwrap();
        let c = ["l", "bk", "linked"].map(|n| File::open(db.join(n)).unwrap());
        let phase = if std::env::var("NWUI_MOVE_PHASE").as_deref() == Ok("restore") { MovePhase::Restore } else { MovePhase::SetAside };
        move_entry(&locks, &mut j, &dirs(&c), &|_| true, &record(phase, StepState::Intent)).unwrap();
        panic!("child missed requested SIGKILL checkpoint");
    }
    fn kill_and_resume(point: &str) {
        for phase in [MovePhase::SetAside, MovePhase::Restore] {
            let f = Fixture::new(); let r = record(phase, StepState::Intent); let ino = seed(&f.path(&r.from));
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "upgrade::moves::tests::sigkill_child", "--nocapture"])
                .env("NWUI_MOVE_FIXTURE", f._tmp.path()).env("NWUI_MOVE_KILL_POINT", point)
                .env("NWUI_MOVE_PHASE", if phase == MovePhase::Restore { "restore" } else { "set-aside" }).output().unwrap();
            assert_eq!(out.status.signal(), Some(libc::SIGKILL), "{phase:?}/{point}: {}", String::from_utf8_lossy(&out.stderr));
            let c = f.caps(); let (locks, mut j) = f.open();
            assert_eq!(j.load().moves.len(), 1); assert_eq!(j.load().moves[0].record.state, StepState::Intent);
            assert_eq!(f.path(&r.from).exists(), point == "intent");
            assert_eq!(f.path(&r.to).exists(), point != "intent");
            move_entry(&locks, &mut j, &dirs(&c), &|_| true, &r).unwrap();
            assert!(!f.path(&r.from).exists()); assert_payload(&f.path(&r.to), ino);
            assert_eq!(j.load().moves.len(), 2); assert_eq!(j.load().moves[1].record.state, StepState::Done);
        }
    }
    #[test] fn sigkill_after_intent() { kill_and_resume("intent"); }
    #[test] fn sigkill_after_rename() { kill_and_resume("rename"); }
    #[test] fn sigkill_between_parent_fsyncs() { kill_and_resume("source-fsync"); }
    #[test] fn sigkill_before_done() { kill_and_resume("destination-fsync"); }
}
