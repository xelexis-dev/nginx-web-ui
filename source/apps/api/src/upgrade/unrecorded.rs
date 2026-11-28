//! Runtime authority when a phase-1 refusal cannot be persisted in the journal.
use super::{Branch, KeyBinding, Outcome, Step, StepError, StorageSelection, status, types::*};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub(super) const MARKER: &str = "/run/nwui-unrecorded.json";
pub(super) const DESCRIPTOR: &str = "/run/nwui-storage.json";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Code {
    JournalWriteFailed,
    ReserveExhausted,
}
impl Code {
    pub(super) fn classify(error: &StepError) -> Option<Self> {
        match error.code {
            "journal_write_failed" => Some(Self::JournalWriteFailed),
            "reserve_exhausted" => Some(Self::ReserveExhausted),
            _ => None,
        }
    }
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::JournalWriteFailed => "journal_write_failed",
            Self::ReserveExhausted => "reserve_exhausted",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum State { Refused, Restarting }

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Marker {
    version: u8,
    boot_identity: String,
    pub state: State,
    pub code: Code,
    pub step: Step,
}

pub(super) fn invalid() -> StepError {
    StepError {
        code: "status_invalid",
        message: "시작 준비를 보류한 기록을 확인하지 못했습니다. 컨테이너 로그를 확인하세요.",
        detail: None,
    }
}

impl Marker {
    pub(super) fn new(identity: String, state: State, code: Code, step: Step) -> Self {
        Self { version: 1, boot_identity: identity, state, code, step }
    }

    pub(super) fn read_at(path: &Path, current: Option<&str>) -> Result<Option<(Self, String)>, StepError> {
        let file = match status::open_runtime_json(path) {
            Ok(file) => file,
            Err(error) if error.code == "status_missing" => return Ok(None),
            Err(error) => return Err(error),
        };
        let result = (|| {
            let (bytes, metadata) = status::read_runtime_json(&file, path)?;
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Identity { boot_identity: String }
            let saved: Identity = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if saved.boot_identity.len() != 64
                || !saved.boot_identity.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                return Err(invalid());
            }
            let current = current.ok_or_else(invalid)?;
            if saved.boot_identity != current { return Ok(None); }
            let marker: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if marker.version != 1 { return Err(invalid()); }
            let updated: chrono::DateTime<chrono::Utc> = metadata.modified().map_err(|_| invalid())?.into();
            Ok(Some((marker, updated.to_rfc3339())))
        })();
        // Uncertain age is treated as current; a proven older record can be cleared by Phase 1.
        if result.is_err() && file.metadata().ok().and_then(|stat|stat.modified().ok()).zip(super::boot_identity::started_at().ok()).is_some_and(|(file,start)|file<start){
            return Ok(None);
        }
        result
    }

    pub(super) fn write_at(&self, path: &Path) -> Result<(), StepError> {
        status::write_runtime_json(path, &serde_json::to_vec(self).map_err(|_| invalid())?)
    }

    pub(super) fn publish_at(&self, descriptor: &Path, marker: &Path, updated: String) -> Result<UpgradeStatus, StepError> {
        super::types::write_selection_at(descriptor, &selection())?;
        #[cfg(test)]
        if MISMATCH_AFTER_WRITE.with(|fault|fault.replace(false)) {
            let mut mismatch=selection();
            mismatch.epoch=2;
            super::types::write_selection_at(descriptor,&mismatch)?;
        }
        check_descriptor(descriptor)?;
        self.write_at(marker)?;
        Ok(self.project(updated))
    }

    pub(super) fn project(&self, updated: String) -> UpgradeStatus {
        let label = status::step_label(self.step);
        let (message, next_action) = match self.code {
            Code::JournalWriteFailed => (
                format!("시작 준비 기록을 저장하지 못해 서비스 시작을 멈췄습니다(실패 단계: {label}). 그 뒤로는 데이터와 복구 기록을 바꾸지 않았습니다."),
                "앱 데이터 볼륨(/var/lib/nginx-web-ui)의 여유 공간과 쓰기 가능 여부를 확인한 뒤 컨테이너를 다시 시작하세요.",
            ),
            Code::ReserveExhausted => (
                format!("앱 데이터 볼륨의 여유 공간이 부족해 시작 준비 기록을 저장하지 못했습니다(실패 단계: {label}). 서비스 시작을 멈췄고 그 뒤로는 데이터와 복구 기록을 바꾸지 않았습니다."),
                "앱 데이터 볼륨(/var/lib/nginx-web-ui)의 여유 공간을 늘린 뒤 컨테이너를 다시 시작하세요.",
            ),
        };
        let code = self.code.as_str();
        UpgradeStatus {
            version: 1, outcome: Outcome::RefusedPreflight, branch: Branch::Upgrade,
            updated_at: updated.clone(),
            step: StatusStep { id: Step::P0, index: 0, total: 0, started_at: updated, bytes_done: 0, bytes_total: 0 },
            failure: Some(StatusFailure { code: code.into(), class: super::class_of(code), message, next_action: next_action.into() }),
            space: None, locations: vec![], backup: None, discarded: vec![], cleanup_pending: false,
            retry: StatusRetry { next_eligible_at: None, reason: None }, warnings: vec![],
        }
    }
}

fn selection() -> StorageSelection {
    StorageSelection {
        version: 1, epoch: 1, outcome: Outcome::RefusedPreflight, branch: Branch::Upgrade,
        pgdata: crate::db_upgrade::paths::PGDATA.into(), backup_id: None, schema_profile: "none".into(),
        key_binding: KeyBinding::Unverified,
    }
}

pub(super) fn check_descriptor(path: &Path) -> Result<(), StepError> {
    if super::types::read_selection_at(path)? != selection() { return Err(invalid()); }
    Ok(())
}

pub(super) fn journal_refusal(value: &UpgradeStatus) -> bool {
    value.outcome == Outcome::RefusedPreflight && value.branch == Branch::Upgrade
        && value.step.id == Step::P0 && value.step.total == 0
        && value.failure.as_ref().is_some_and(|f| matches!(f.code.as_str(), "journal_write_failed" | "reserve_exhausted"))
}

#[cfg(test)]
thread_local! {
    static MISMATCH_AFTER_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn marker_is_boot_bound_and_projects_without_status_or_journal() {
        let temp = tempfile::tempdir().unwrap();
        let marker_path = temp.path().join("marker");
        let descriptor = temp.path().join("selection");
        let identity = "a".repeat(64);
        for state in [State::Restarting, State::Refused] {
            let marker = Marker::new(identity.clone(), state, Code::JournalWriteFailed, Step::Handoff);
            marker.publish_at(&descriptor, &marker_path, "2026-11-24T00:00:00Z".into()).unwrap();
            let (actual, at) = Marker::read_at(&marker_path, Some(&identity)).unwrap().unwrap();
            assert_eq!(actual.state,state);
            assert_eq!(actual.step,Step::Handoff);
            assert!(Marker::read_at(&marker_path, Some(&"b".repeat(64))).unwrap().is_none());
            assert!(Marker::read_at(&marker_path, None).is_err());
            let value = actual.project(at);
            assert_eq!((value.outcome,value.branch,value.step.id,value.step.total),(Outcome::RefusedPreflight,Branch::Upgrade,Step::P0,0));
            assert!(status::format_human(&value).starts_with("시작 준비 기록을 저장하지 못했습니다\n"));
            let failure=value.failure.unwrap();
            assert!(failure.message.contains("서비스 시작"));
            assert!(!failure.message.contains("여유 공간이 부족"));
        }
        let mut wrong = selection();
        wrong.epoch = 2;
        super::super::types::write_selection_at(&descriptor, &wrong).unwrap();
        assert!(check_descriptor(&descriptor).is_err());
        fs::write(&marker_path,format!(r#"{{"bootIdentity":"{}","state":"unknown"}}"#,"b".repeat(64))).unwrap();
        assert!(Marker::read_at(&marker_path,Some(&identity)).unwrap().is_none());
        fs::write(&marker_path,b"{").unwrap();
        assert!(Marker::read_at(&marker_path,Some(&identity)).is_err());
    }

    #[test]
    fn a_malformed_or_identity_less_marker_is_ignored_only_if_older_than_pid_one() {
        use std::{fs::File, time::{Duration,SystemTime}};
        let temp=tempfile::tempdir().unwrap();
        let path=temp.path().join("marker");
        let start=super::super::boot_identity::started_at().unwrap();
        let identity="a".repeat(64);
        for (text,current) in [("{".to_owned(),Some(identity.as_str())),
            (serde_json::to_string(&Marker::new(identity.clone(),State::Refused,Code::JournalWriteFailed,Step::Y1)).unwrap(),None)] {
            fs::write(&path,text).unwrap();
            let file=File::options().write(true).open(&path).unwrap();
            file.set_modified(start-Duration::from_secs(2)).unwrap();
            assert!(Marker::read_at(&path,current).unwrap().is_none());
            file.set_modified(SystemTime::now()+Duration::from_secs(1)).unwrap();
            assert_eq!(Marker::read_at(&path,current).unwrap_err().code,"status_invalid");
        }
    }

    #[test]
    fn refusal_requires_both_runtime_records() {
        let temp = tempfile::tempdir().unwrap();
        let marker_path = temp.path().join("marker");
        let descriptor = temp.path().join("selection");
        let marker = Marker::new("a".repeat(64), State::Refused, Code::ReserveExhausted, Step::Y1);
        fs::create_dir(&descriptor).unwrap();
        assert!(marker.publish_at(&descriptor,&marker_path,String::new()).is_err());
        assert!(!marker_path.exists());
        fs::remove_dir(&descriptor).unwrap();
        MISMATCH_AFTER_WRITE.with(|fault|fault.set(true));
        assert!(marker.publish_at(&descriptor,&marker_path,String::new()).is_err());
        assert!(!marker_path.exists());
        fs::create_dir(&marker_path).unwrap();
        assert!(marker.publish_at(&descriptor,&marker_path,String::new()).is_err());
        assert!(check_descriptor(&descriptor).is_ok());
    }

    #[test]
    fn browser_uses_the_same_title_message_and_next_action() {
        let js = include_str!("../../../../docker/upgrade-status/status.mjs");
        for code in [Code::JournalWriteFailed,Code::ReserveExhausted] {
            let marker=Marker::new("a".repeat(64),State::Refused,code,Step::Handoff);
            let value=marker.project(String::new());
            let cli=status::format_human(&value);
            assert!(js.contains(cli.lines().next().unwrap()));
            let failure=value.failure.unwrap();
            assert!(js.contains(&failure.message.replace("서비스 시작).","${label}).")));
            assert!(js.contains(&failure.next_action));
        }
    }
}
