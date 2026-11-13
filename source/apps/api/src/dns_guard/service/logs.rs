use super::super::VALID_SECONDS;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
const MAX_LOGS: usize = 32;
const MAX_KEYS: usize = 128;
pub(super) struct Budget {
    window: Instant,
    remaining: usize,
    last: HashMap<String, Instant>,
}
impl Budget {
    pub(super) fn new() -> Self {
        Self {
            window: Instant::now(),
            remaining: MAX_LOGS,
            last: HashMap::new(),
        }
    }
    pub(super) fn line(
        &mut self,
        category: &str,
        subject: &str,
        reason: &str,
        now: Instant,
    ) -> Option<String> {
        let duration = Duration::from_secs(VALID_SECONDS.into());
        if now.saturating_duration_since(self.window) >= duration {
            self.window = now;
            self.remaining = MAX_LOGS;
        }
        self.last
            .retain(|_, at| now.saturating_duration_since(*at) < duration);
        let category = bounded(category, 32);
        let subject = bounded(subject, 255);
        let key = format!("{category}\0{subject}");
        if self.remaining == 0 || self.last.contains_key(&key) || self.last.len() >= MAX_KEYS {
            return None;
        }
        self.remaining -= 1;
        self.last.insert(key, now);
        Some(format!(
            "[dns-guard] {category} {subject}: {}",
            bounded(reason, 255)
        ))
    }
}
fn bounded(raw: &str, max: usize) -> String {
    let mut result = String::with_capacity(raw.len().min(max));
    for mut c in raw.chars() {
        if c.is_control() {
            c = '?';
        }
        if result.len() + c.len_utf8() > max {
            break;
        }
        result.push(c);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_rate_limited_across_global_window_boundaries() {
        let mut b = Budget::new();
        let now = Instant::now();
        assert!(b.line("rejected", "same.test", "unsafe", now).is_some());
        assert!(
            b.line(
                "rejected",
                "same.test",
                "unsafe",
                now + Duration::from_secs(1)
            )
            .is_none()
        );
        assert!(
            b.line(
                "rejected",
                "late.test",
                "unsafe",
                now + Duration::from_secs(9)
            )
            .is_some()
        );
        assert!(
            b.line(
                "rejected",
                "late.test",
                "unsafe",
                now + Duration::from_secs(10)
            )
            .is_none()
        );
        assert!(
            b.line(
                "rejected",
                "same.test",
                "unsafe",
                now + Duration::from_secs(10)
            )
            .is_some()
        );
    }
    #[test]
    fn unique_name_flood_has_bounded_output_and_memory() {
        let mut b = Budget::new();
        let now = Instant::now();
        let mut accepted = 0;
        for n in 0..10_000 {
            accepted += usize::from(
                b.line("rejected", &format!("{n}.test"), "unsafe", now)
                    .is_some(),
            );
        }
        assert!(accepted <= MAX_LOGS);
        assert!(b.last.len() <= MAX_KEYS);
        assert!(
            b.line(
                "rejected",
                "new.test",
                "unsafe",
                now + Duration::from_secs(VALID_SECONDS.into())
            )
            .is_some()
        );
    }
    #[test]
    fn log_fields_are_byte_bounded_utf8_and_single_line() {
        let mut b = Budget::new();
        let now = Instant::now();
        let line = b
            .line(
                "rejected",
                &format!("{}\\n\n", "한".repeat(1000)),
                &"why\n".repeat(1000),
                now,
            )
            .unwrap();
        assert!(line.len() <= 600);
        assert!(!line.chars().any(char::is_control));
        assert!(b.last.keys().all(|key| key.len() <= 300));
    }
}
