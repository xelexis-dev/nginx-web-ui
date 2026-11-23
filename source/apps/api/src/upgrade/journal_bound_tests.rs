use super::*;
use crate::db_upgrade::layout::Roots;
use crate::upgrade::{commands, generation, operator, refusal, retry, rollover, runner, selector, types};
use serde_json::{json, Value};
use std::{cell::Cell, os::unix::fs::PermissionsExt};

thread_local! {
    pub(super) static INVALID_FOLD: Cell<bool> = const { Cell::new(false) };
    pub(super) static FAIL_AFTER_RENAME: Cell<bool> = const { Cell::new(false) };
}

const GEN: &str = "00000000-0000-4000-8000-000000000001";
const OTHER_GEN: &str = "00000000-0000-4000-8000-000000000002";
const AT: &str = "2026-11-20T00:00:00Z";
const LIMIT: usize = 1_048_576;

fn fixture(branch: Branch) -> (tempfile::TempDir, Roots, StorageLocks, Journal) {
    let temp = tempfile::tempdir().unwrap();
    let roots = Roots { db: temp.path().join("db"), app: temp.path().join("app"), architecture: "arm64".into() };
    for path in [&roots.db, &roots.app] {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let locks = StorageLocks::acquire(&roots.db, &roots.app).unwrap();
    let j = Journal::begin(&roots.app, &locks, branch, Candidate {
        revision: "0".repeat(40), pins_sha256: "1".repeat(64), migrations_sha256: "2".repeat(64),
    }).unwrap();
    if branch == Branch::Upgrade {
        j.ensure_app_pool().unwrap();
    }
    (temp, roots, locks, j)
}

fn test_name(name: &str) -> String {
    let (_, module) = module_path!().split_once("::").unwrap();
    format!("{module}::{name}")
}

fn bytes(value: &impl Serialize) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

fn add(j: &mut JournalV1, step: Step, state: StepState, data: Option<Value>) {
    let last = j.records.last().map_or(0, |r| r.seq).max(j.moves.last().map_or(0, |m| m.seq));
    j.records.push(Record { seq: last + 1, step, state, at: AT.into(), data });
}

fn conversion(j: &mut JournalV1) {
    add(j, Step::P1Backup, StepState::Done, Some(json!({"sourceBytes": 2048})));
    add(j, Step::P4Target, StepState::Done, Some(json!({"bootStep": "run_migrations"})));
    add(j, Step::P5Commit, StepState::Intent, Some(json!({"authorityEvent": "selector-intent", "generation": GEN, "selectorSha256": "4".repeat(64)})));
    add(j, Step::P5Commit, StepState::Done, None);
}

fn prepare(j: &JournalV1, boot: u64) -> Vec<Record> {
    let selection = types::StorageSelection {
        version: 1, epoch: boot, outcome: Outcome::Pg17, branch: j.branch,
        pgdata: "/var/lib/postgresql/data/17/data".into(), backup_id: None,
        schema_profile: "2.1.2".into(), key_binding: types::KeyBinding::Verified { pair_id: GEN.into() },
    };
    crate::nonroot::fixture_prepare_records(&selection, &j.attempt_id, 1, true)
}

type BootRecord = (Step, StepState, Option<Value>);

fn boot_records(j: &JournalV1, boot: u64, generation: &str, through_backfill: bool) -> Vec<BootRecord> {
    use Step::*;
    use StepState::*;
    let step = if j.branch == Branch::Upgrade { P4Target } else { Y2 };
    let id = format!("{boot:064x}");
    let mut rows = Vec::new();
    for name in ["run_migrations", "preflight_api_port_rows", "ensure_instance", "seed_admin", "dns_preflight", "backfill"] {
        rows.extend([Intent, Done].map(|state| (step, state, Some(json!({"bootStep": name})))));
    }
    if through_backfill {
        return rows;
    }
    if j.branch == Branch::Upgrade {
        rows.extend([Intent, Done].map(|state| (step, state, Some(json!({"bootStep": "upgrade_notifications"})))));
    }
    let privileges = Some(json!({"bootStep": "prepare_privileges"}));
    rows.push((P5Publish, Intent, privileges.clone()));
    rows.extend(prepare(j, boot).into_iter().map(|r| (r.step, r.state, r.data)));
    rows.push((P5Publish, Done, privileges));
    rows.extend([Intent, Done].map(|state| (S4Activate, state, None)));
    rows.extend([Intent, Done].map(|state| (Handoff, state, Some(json!({"bootIdentity": id})))));
    rows.push((Phase2Monitor, Intent, None));
    rows.push((Phase2Monitor, Done, Some(json!({"bootIdentity": id, "generation": generation, "observedAt": AT}))));
    rows
}

fn append(j: &mut JournalV1, (step, state, data): (Step, StepState, Option<Value>)) {
    if step == Step::Handoff {
        j.outcome = Outcome::Pg17;
        j.handoff = if state == StepState::Done { Handoff::Done } else { Handoff::Intent };
    }
    add(j, step, state, data);
}

fn folded(j: &JournalV1) -> JournalV1 {
    fold(j).unwrap_or_else(|| j.clone())
}

fn json_result<T: Serialize>(result: Result<T, StepError>) -> Result<Value, &'static str> {
    result.map(|valid| serde_json::to_value(valid).unwrap()).map_err(|error| error.code)
}

fn compare(roots: &Roots, full: &JournalV1, compact: &JournalV1, ids: &[&str], generations: &[&str]) {
    let now = AT.parse().unwrap();
    full.validate().unwrap();
    compact.validate().unwrap();
    let mut full_top = full.clone();
    let mut compact_top = compact.clone();
    full_top.records.clear();
    compact_top.records.clear();
    assert_eq!(bytes(&full_top), bytes(&compact_top));
    assert_eq!(bytes(&full.records.last()), bytes(&compact.records.last()));
    assert_eq!(last_attempt_failure(&full.records), last_attempt_failure(&compact.records));
    assert_eq!(commands_settled(&full.records), commands_settled(&compact.records));
    assert_eq!(json_result(rollover::retention(full)), json_result(rollover::retention(compact)));
    for label in [None, Some("new-label")] {
        for space in [false, true] {
            assert_eq!(retry::admit(full, &full.candidate, label, space, now).unwrap(),
                retry::admit(compact, &full.candidate, label, space, now).unwrap());
        }
    }
    for step in [Step::S4Activate, Step::Y2, Step::P4Target] {
        assert_eq!(runner::committed_failure_outcome(full, step), runner::committed_failure_outcome(compact, step));
    }
    let root = roots.app.join(".nwui-auto");
    let full_j = Journal { root: root.clone(), state: full.clone() };
    let compact_j = Journal { root: root.clone(), state: compact.clone() };
    assert_eq!(full_j.next_seq().unwrap(), compact_j.next_seq().unwrap());
    for split in [true, false] {
        assert_eq!(bytes(&generation::completed_preparation_for(&full_j, split)),
            bytes(&generation::completed_preparation_for(&compact_j, split)));
    }
    for step in [Step::CmdRemoveBackup, Step::CmdRemoveDiscarded] {
        let data = if step == Step::CmdRemoveBackup { json!({"backupId": full.attempt_id}) } else { json!({"discardId": OTHER_GEN}) };
        assert_eq!(operator::progress(full, step, &data).map_err(|e| e.code),
            operator::progress(compact, step, &data).map_err(|e| e.code));
    }
    for id in ids {
        assert_eq!(refusal::outcome(full, id), refusal::outcome(compact, id));
        assert_eq!(runner::boot_plan(full, Some(id)), runner::boot_plan(compact, Some(id)));
        for g in generations {
            assert_eq!(runner::activation_proof(&full_j, id, g), runner::activation_proof(&compact_j, id, g));
        }
    }
    for g in generations {
        let selected = selector::Selector {
            version: 1, active_major: 17, generation: Some((*g).into()), system_identifier: "123456".into(),
            origin: selector::Origin::Auto, backup_id: Some(full.attempt_id.clone()), profile_id: Some("legacy".into()),
            profile_sha256: Some("3".repeat(64)), manifest_sha256: Some("4".repeat(64)), operator_reason: None,
        };
        assert_eq!(operator::started_once(full, &selected).map_err(|e| e.code),
            operator::started_once(compact, &selected).map_err(|e| e.code));
        assert_eq!(operator::retained_attempt(full, &selected, OTHER_GEN).map_err(|e| e.code),
            operator::retained_attempt(compact, &selected, OTHER_GEN).map_err(|e| e.code));
    }
    for step in full.records.iter().map(|r| r.step).collect::<std::collections::HashSet<_>>() {
        for intent_only in [false, true] {
            let latest = |j: &JournalV1| bytes(&j.records.iter().rev().find(|r|
                r.step == step && (!intent_only || r.state == StepState::Intent)));
            assert_eq!(latest(full), latest(compact));
        }
    }
    assert_eq!(json_result(refusal::status_value(roots, full, full.outcome, &full.candidate, None)),
        json_result(refusal::status_value(roots, compact, compact.outcome, &full.candidate, None)));
    let render = |j| {
        fs::write(root.join("journal-v1.json"), bytes(j)).unwrap();
        let offline = json_result(commands::offline_status(roots, &full.candidate, now));
        let human = commands::render_at(&roots.app.join("missing-live-status"), &roots.app).map_err(|e| e.code);
        (offline, human)
    };
    assert_eq!(render(full), render(compact));
}

#[test]
fn every_prefix_preserves_readers_across_partial_boots_and_commands() {
    for branch in [Branch::Fresh, Branch::Ready, Branch::Upgrade] {
        let (_t, roots, _locks, j) = fixture(branch);
        let mut full = j.state.clone();
        if branch == Branch::Upgrade {
            conversion(&mut full);
        }
        let mut compact = full.clone();
        let mut seed = 0x27d4eb2du32;
        for boot in 1..=24 {
            let id = format!("{boot:064x}");
            let prior = format!("{:064x}", boot - 1);
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let rows = boot_records(&full, boot, if boot % 3 == 0 { OTHER_GEN } else { GEN }, false);
            let kill_at = if boot % 4 == 0 { rows.len() } else { 1 + seed as usize % rows.len() };
            let mut transition = |row: BootRecord| {
                append(&mut full, row.clone());
                append(&mut compact, row);
                compact = folded(&compact);
                compare(&roots, &full, &compact, &[&id, &prior], &[GEN, OTHER_GEN]);
            };
            for row in rows.into_iter().take(kill_at) {
                transition(row);
            }
            if boot % 4 == 1 {
                transition((Step::P5Publish, StepState::Done, Some(json!({"authorityEvent":"privilege-discarded"}))));
            }
            if boot % 4 == 2 {
                transition((Step::P5Publish, StepState::Done, Some(json!({"authorityEvent":"privilege-split-off"}))));
            }
            if boot % 6 == 0 {
                let data = Some(json!({"discardId": OTHER_GEN}));
                transition((Step::CmdRemoveDiscarded, StepState::Intent, data.clone()));
                transition((Step::CmdRemoveDiscarded, StepState::Done, data));
            }
            if boot % 3 == 1 {
                let outcome = Outcome::RefusedPrivilege;
                full.outcome = outcome;
                compact.outcome = outcome;
                for j in [&mut full, &mut compact] {
                    add(j, Step::Y1, StepState::Failed { code: format!("failure_{boot}") }, None);
                    add(j, Step::P5Publish, StepState::Done, Some(refusal::receipt(outcome, &id).unwrap()));
                }
                compact = folded(&compact);
                compare(&roots, &full, &compact, &[&id, &prior], &[GEN, OTHER_GEN]);
            }
        }
        assert!(compact.records.len() < full.records.len());
    }
    if let Some(path) = std::env::var_os("NWUI_JOURNAL_FIXTURES") {
        real_copies_preserve_readers_and_report_sizes(Path::new(&path));
    }
}

#[test]
fn extended_prepare_and_conversion_prefix_remain_byte_identical() {
    let (_t, roots, _locks, j) = fixture(Branch::Ready);
    let mut full = j.state.clone();
    let data = prepare(&full, 1)[0].data.clone().unwrap();
    for _ in 0..2 {
        let mut extended = data.clone();
        extended["space"] = json!({"requiredBytes":4096,"availableBytes":1024,"inodesRequired":8,"inodesAvailable":4});
        add(&mut full, Step::S4Prepare, StepState::Intent, Some(extended));
        add(&mut full, Step::S4Prepare, StepState::Intent, Some(data.clone()));
    }
    add(&mut full, Step::Y1, StepState::Failed { code: "verification_failed".into() }, None);
    full.outcome = Outcome::RefusedPrivilege;
    let compact = fold(&full).unwrap();
    let space_rows = |j: &JournalV1| j.records.iter().filter(|r| r.data.as_ref().is_some_and(|d| d.get("space").is_some())).map(bytes).collect::<Vec<_>>();
    assert_eq!(space_rows(&full), space_rows(&compact));
    compare(&roots, &full, &compact, &[], &[GEN]);
    let rendered = commands::render_at(&roots.app.join("missing-live-status"), &roots.app).unwrap();
    assert!(rendered.contains("공간: 1024 / 4096 바이트, inode: 4 / 8"));
}

#[test]
fn guard_and_exact_payloads_leave_unrecognized_or_unsettled_frames_alone() {
    let (_t, _roots, _locks, j) = fixture(Branch::Upgrade);
    let mut full = j.state.clone();
    assert!(pre_pool_frame(&full));
    assert!(fold(&full).is_none());
    for _ in 0..2 {
        add(&mut full, Step::P4Target, StepState::Done, Some(json!({"bootStep":"backfill"})));
    }
    add(&mut full, Step::P5Commit, StepState::Done, Some(json!({"unrecognized":true})));
    assert!(fold(&full).is_none());
    conversion(&mut full);
    let prefix = bytes(&full.records);
    for _ in 0..2 {
        add(&mut full, Step::P4Target, StepState::Done, Some(json!({"bootStep":"backfill"})));
        add(&mut full, Step::P4Target, StepState::Done, Some(json!({"bootStep":"future_boot_step"})));
        let mut extended = prepare(&full, 1)[0].data.clone().unwrap();
        extended["logModes"][0]["futureField"] = json!(true);
        add(&mut full, Step::S4Prepare, StepState::Intent, Some(extended));
    }
    let compact = fold(&full).unwrap();
    assert_eq!(compact.records.len() + 1, full.records.len());
    assert_eq!(bytes(&&compact.records[..full.records.len() - 6]), prefix);

    for outcome in [Outcome::Restored, Outcome::RestoreIncomplete] {
        let mut excluded = full.clone();
        excluded.outcome = outcome;
        assert!(fold(&excluded).is_none());
    }
    for command in [Step::CmdRestorePg15, Step::CmdRemoveBackup, Step::CmdRemoveDiscarded] {
        let mut excluded = full.clone();
        add(&mut excluded, command, StepState::Intent, None);
        assert!(fold(&excluded).is_none());
    }
    let next_seq = full.records.last().unwrap().seq + 1;
    full.moves.push(JournalMove { seq: next_seq, record: MoveRecord {
        phase: MovePhase::SetAside, entry: "base".into(), from: "L".into(), to: "BK".into(), state: StepState::Intent,
    }});
    assert!(fold(&full).is_none());
    let mut done = full.moves.last().unwrap().clone();
    done.seq += 1;
    done.record.state = StepState::Done;
    full.moves.push(done);
    let compact = fold(&full).unwrap();
    assert_eq!(bytes(&full.moves), bytes(&compact.moves));
    assert_eq!(compact.moves.last().unwrap().seq, next_seq + 1);
}

#[test]
fn three_hundred_completed_boots_and_postcommit_kills_stay_bounded() {
    for (branch, partial) in [(Branch::Fresh, false), (Branch::Upgrade, false), (Branch::Upgrade, true)] {
        let (_t, _roots, _locks, j) = fixture(branch);
        let mut frame = j.state.clone();
        if branch == Branch::Upgrade {
            conversion(&mut frame);
        }
        let prefix = bytes(&frame.records);
        let prefix_len = frame.records.len();
        let mut steady_count = 0;
        for boot in 1..=300 {
            for row in boot_records(&frame, boot, GEN, partial) {
                append(&mut frame, row);
                if let Some(small) = fold(&frame) {
                    frame = small;
                }
                frame.validate().unwrap();
                assert!(bytes(&frame).len() < 64 * 1024);
            }
            if boot == 2 {
                steady_count = frame.records.len();
            }
            if boot >= 2 {
                assert_eq!(frame.records.len(), steady_count);
            }
        }
        assert_eq!(bytes(&&frame.records[..prefix_len]), prefix);
        if partial {
            assert_eq!(frame.outcome, Outcome::Upgrading);
        }
        eprintln!("journal bound {branch:?}, partial={partial}: {} B / {steady_count} records", bytes(&frame).len());
    }
}

/// Use short genuine boot records, leaving too little room for a larger last publication.
pub(crate) fn inflate(j: &mut JournalV1) {
    let step = if j.branch == Branch::Upgrade { Step::P4Target } else { Step::Y2 };
    let mut size = bytes(j).len();
    loop {
        let extra = usize::from(!j.records.is_empty());
        add(j, step, StepState::Done, Some(json!({"bootStep":"backfill"})));
        let added = bytes(j.records.last().unwrap()).len() + extra;
        if size + added > LIMIT {
            j.records.pop();
            break;
        }
        size += added;
    }
    assert!(bytes(j).len() > LIMIT - 130);
}

#[test]
fn first_write_to_near_limit_frame_folds_before_serialized_size_check() {
    let (_t, roots, locks, mut j) = fixture(Branch::Fresh);
    let mut full = j.state.clone();
    for row in boot_records(&full, 1, GEN, false) {
        append(&mut full, row);
    }
    inflate(&mut full);
    let before_size = bytes(&full).len();
    fs::write(j.root.join("journal-v1.json"), bytes(&full)).unwrap();
    j = Journal::open(&roots.app, &locks).unwrap();
    let data = Some(json!({"bootStep":"prepare_privileges"}));
    j.record(Step::P5Publish, StepState::Intent, data.clone()).unwrap();
    full.records.push(j.state.records.last().unwrap().clone());
    assert!(bytes(&full).len() > LIMIT);
    let disk = Journal::open(&roots.app, &locks).unwrap();
    assert_eq!(bytes(&disk.state), bytes(&j.state));
    // The oversized unfolded proposal cannot be read from disk, but its pure readers still agree.
    assert_eq!(bytes(&fold(&full).unwrap()), bytes(&disk.state));
    assert!(bytes(&disk.state).len() < 64 * 1024);
    eprintln!("at-limit first write: {before_size} B → {} B", bytes(&disk.state).len());
}

#[test]
fn invalid_fold_persists_full_frame_and_logs_once() {
    if std::env::var_os("NWUI_JOURNAL_INVALID_FOLD_CHILD").is_some() {
        let (_t, roots, locks, mut j) = fixture(Branch::Fresh);
        let mut expected = j.state.clone();
        for row in boot_records(&expected, 1, GEN, false) {
            append(&mut expected, row);
        }
        add(&mut expected, Step::Y2, StepState::Intent, Some(json!({"bootStep":"backfill"})));
        assert!(fold(&expected).is_some());
        INVALID_FOLD.set(true);
        j.commit(expected.clone()).unwrap();
        assert_eq!(bytes(&j.state), bytes(&expected));
        assert_eq!(bytes(&Journal::open(&roots.app, &locks).unwrap().state), bytes(&expected));
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name("invalid_fold_persists_full_frame_and_logs_once").as_str(), "--nocapture"])
        .env("NWUI_JOURNAL_INVALID_FOLD_CHILD", "1").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stderr).lines()
        .filter(|line| *line == "[upgrade] journal compaction skipped").count(), 1);
}

#[test]
fn kill_during_handoff_keeps_readable_before_or_after_transition() {
    use std::os::unix::process::ExitStatusExt;
    if let Some(base) = std::env::var_os("NWUI_JOURNAL_HANDOFF_CHILD") {
        let app = Path::new(&base).join("app");
        let db = Path::new(&base).join("db");
        let locks = StorageLocks::acquire(&db, &app).unwrap();
        Journal::open(&app, &locks).unwrap().handoff(Handoff::Done, &format!("{:064x}", 3)).unwrap();
        panic!("kill point not reached");
    }
    let id = format!("{:064x}", 3);
    for (point, renamed) in [("slot-written", false), ("slot-fsynced", false), ("journal-renamed", true), ("journal-durable", true)] {
        let (t, roots, locks, mut j) = fixture(Branch::Upgrade);
        let mut reference = j.state.clone();
        conversion(&mut reference);
        for boot in 1..=2 {
            for row in boot_records(&reference, boot, GEN, false) {
                append(&mut reference, row);
            }
        }
        append(&mut reference, (Step::Handoff, StepState::Intent, Some(json!({"bootIdentity": id}))));
        j.commit(reference.clone()).unwrap();
        drop(locks);
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name("kill_during_handoff_keeps_readable_before_or_after_transition").as_str(), "--nocapture"])
            .env("NWUI_JOURNAL_HANDOFF_CHILD", t.path()).env("NWUI_JOURNAL_KILL_POINT", point)
            .output().unwrap();
        assert_eq!(out.status.signal(), Some(libc::SIGKILL), "{point}: {}", String::from_utf8_lossy(&out.stderr));
        let locks = StorageLocks::acquire(&roots.db, &roots.app).unwrap();
        let disk = Journal::open(&roots.app, &locks).unwrap();
        if renamed {
            reference.handoff = Handoff::Done;
            reference.records.push(disk.state.records.last().unwrap().clone());
        }
        assert_eq!(runner::boot_plan(&disk.state, Some(&id)),
            if renamed { runner::BootPlan::Phase2 } else { runner::BootPlan::Phase1 });
        compare(&roots, &reference, &disk.state, &[&id], &[GEN]);
    }
}

#[test]
fn post_rename_error_and_stale_monitor_retry_match_unfolded_frames() {
    let (_t, roots, locks, mut j) = fixture(Branch::Upgrade);
    let id = format!("{:064x}", 2);
    let mut before = j.state.clone();
    conversion(&mut before);
    for row in boot_records(&before, 1, GEN, false) {
        append(&mut before, row);
    }
    for row in boot_records(&before, 2, GEN, false) {
        if row.0 == Step::Phase2Monitor && row.1 == StepState::Done {
            break;
        }
        append(&mut before, row);
    }
    j.commit(before.clone()).unwrap();
    let stale = bytes(&j.state);
    let data = Some(json!({"bootIdentity": id, "generation": GEN, "observedAt": AT}));
    FAIL_AFTER_RENAME.set(true);
    let error = j.record(Step::Phase2Monitor, StepState::Done, data.clone()).unwrap_err();
    assert_eq!(error.code, "journal_write_failed");
    assert_eq!(bytes(&j.state), stale);
    let disk = Journal::open(&roots.app, &locks).unwrap();
    let mut after = before.clone();
    after.records.push(disk.state.records.last().unwrap().clone());
    // The oracle writes its own diagnostic fixture, never the journal used for retry.
    let (_oracle, oracle_roots, _oracle_locks, _) = fixture(Branch::Ready);
    compare(&oracle_roots, &after, &disk.state, &[&id], &[GEN]);
    assert!(runner::activation_proof(&disk, &id, GEN));
    assert!(!runner::activation_proof(&j, &id, GEN));
    j.record(Step::Phase2Monitor, StepState::Intent, None).unwrap();
    before.records.push(j.state.records.last().unwrap().clone());
    let disk = Journal::open(&roots.app, &locks).unwrap();
    assert_eq!(bytes(&disk.state), bytes(&j.state));
    compare(&oracle_roots, &before, &disk.state, &[&id], &[GEN]);
    j.record(Step::Phase2Monitor, StepState::Done, data).unwrap();
    before.records.push(j.state.records.last().unwrap().clone());
    let disk = Journal::open(&roots.app, &locks).unwrap();
    assert_eq!(bytes(&disk.state), bytes(&j.state));
    compare(&oracle_roots, &before, &disk.state, &[&id], &[GEN]);
}

fn real_copies_preserve_readers_and_report_sizes(path: &Path) {
    fn walk(root: &Path, visit: &mut impl FnMut(&Path)) {
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, visit);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                visit(&path);
            }
        }
    }
    let (_t, roots, _locks, _) = fixture(Branch::Ready);
    for case in ["s1", "s2", "s3", "s4"] {
        let mut count = 0;
        walk(&path.join(case), &mut |p| {
            let original = fs::read(p).unwrap();
            let Ok(full) = serde_json::from_slice::<JournalV1>(&original) else { return };
            let compact = folded(&full);
            let ids = full.records.iter().rev().filter_map(|r| r.data.as_ref()?.get("bootIdentity")?.as_str())
                .take(3).collect::<Vec<_>>();
            let generations = full.records.iter().filter_map(|r| r.data.as_ref()?.get("generation")?.as_str())
                .collect::<std::collections::HashSet<_>>().into_iter().collect::<Vec<_>>();
            compare(&roots, &full, &compact, &ids, &generations);
            if full.branch == Branch::Upgrade {
                let prefix = full.records.iter().position(|r| r.step == Step::P5Commit && r.state == StepState::Done && r.data.is_none()).unwrap();
                assert_eq!(bytes(&&full.records[..=prefix]), bytes(&&compact.records[..=prefix]));
            }
            if ["cur.json", "after-203.json", "final.json"].iter().any(|name| p.file_name().unwrap() == *name) {
                eprintln!("journal {case}/{}: {} B / {} → {} B / {} records",
                    p.file_name().unwrap().to_string_lossy(), original.len(), full.records.len(), bytes(&compact).len(), compact.records.len());
            }
            assert!(bytes(&compact).len() < 64 * 1024);
            count += 1;
        });
        assert!(count > 0, "{case}");
        eprintln!("checked {count} {case} journal snapshots");
    }
}
