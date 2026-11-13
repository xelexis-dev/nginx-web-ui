//! Gateway ↔ node compatibility, a port of `packages/node-api/src/version.ts`.
//!
//! Arguments are (gateway, node). For the same major ≥ 2, the gateway minor must be at least
//! the node minor. Major 1 keeps exact major.minor matching; 2.0 ↔ 1.19 is the supported bridge.
//! Patches preserve the node contract; newer minors must remain additive for older nodes.
//! Unreadable on either side means incompatible — refusing is the default.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Semver {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

/// `/^(\d+)\.(\d+)\.(\d+)/` after an ECMAScript `trim()` — a prefix match, so `2.0.0-rc.1` parses.
pub fn parse_semver(value: &str) -> Option<Semver> {
    let mut rest = crate::crypto::js_trim(value);
    let mut number = |last: bool| -> Option<u64> {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let parsed = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        if !last {
            rest = rest.strip_prefix('.')?;
        }
        Some(parsed)
    };
    Some(Semver { major: number(false)?, minor: number(false)?, patch: number(true)? })
}

pub fn is_compatible(gateway: &str, node: &str) -> bool {
    match (parse_semver(gateway), parse_semver(node)) {
        (Some(g), Some(n)) => (g.major == n.major
            && ((g.major >= 2 && g.minor >= n.minor) || (g.major == 1 && g.minor == n.minor)))
            || matches!(((g.major, g.minor), (n.major, n.minor)), ((2, 0), (1, 19)) | ((1, 19), (2, 0))),
        _ => false,
    }
}

/// One human-readable line saying which side to upgrade (verbatim TS text).
pub fn describe_mismatch(gateway: &str, node: &str) -> String {
    let Some(n) = parse_semver(node) else {
        let shown = if gateway.is_empty() { "알 수 없음" } else { gateway };
        return format!("이 노드에는 노드 기능이 없습니다. 노드를 업그레이드하세요. (게이트웨이 {shown})");
    };
    let Some(g) = parse_semver(gateway) else {
        return format!("게이트웨이 버전을 알 수 없습니다. (노드 {node})");
    };
    if (n.major, n.minor) < (g.major, g.minor) {
        format!("노드 버전이 낮습니다 — 노드를 올리세요. (게이트웨이 {gateway} / 노드 {node})")
    } else {
        format!("게이트웨이 버전이 낮습니다 — 게이트웨이를 올리세요. (게이트웨이 {gateway} / 노드 {node})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_semver_prefix_after_trim() {
        assert_eq!(parse_semver("1.15.3"), Some(Semver { major: 1, minor: 15, patch: 3 }));
        assert_eq!(parse_semver("  1.15.3\n"), Some(Semver { major: 1, minor: 15, patch: 3 }));
        assert_eq!(parse_semver("2.0.0-rc.1"), Some(Semver { major: 2, minor: 0, patch: 0 }));
        for bad in ["", "1.15", "v1.2.3", "1.x.3", " .1.2", "１.2.3"] {
            assert_eq!(parse_semver(bad), None, "{bad:?}");
        }
    }

    fn versions() -> Vec<String> {
        let mut values = Vec::new();
        for family in ["1.17", "1.18", "1.19", "1.20", "2.0", "2.1", "2.9", "3.0"] {
            for patch in [0, 9] { values.push(format!("{family}.{patch}")); }
        }
        values.extend(["2.1.0-rc.1".into(), "".into(), "junk".into()]);
        values
    }

    #[test]
    fn shipped_pairs_keep_the_legacy_rule() {
        fn legacy_rule(gateway: &str, node: &str) -> bool {
            match (parse_semver(gateway), parse_semver(node)) {
                (Some(g), Some(n)) => (g.major == n.major && g.minor == n.minor)
                    || matches!(((g.major, g.minor), (n.major, n.minor)), ((2, 0), (1, 19)) | ((1, 19), (2, 0))),
                _ => false,
            }
        }
        let shipped: Vec<_> = versions().into_iter().filter(|v| parse_semver(v).is_some_and(|v| v.major == 1 || (v.major == 2 && v.minor == 0))).collect();
        for g in &shipped {
            for n in &shipped { assert_eq!(is_compatible(g, n), legacy_rule(g, n), "{g} -> {n}"); }
        }
        for patch in 0..=11 {
            let legacy = format!("1.19.{patch}");
            assert!(is_compatible("2.0.0", &legacy));
            assert!(is_compatible(&legacy, "2.0.9"));
        }
    }

    #[test]
    fn gateway_first_version_grid() {
        // Explicit family edges keep the oracle independent of the implementation's inequality.
        let modern_edges = [(0, 0), (1, 0), (1, 1), (9, 0), (9, 1), (9, 9)];
        for gateway in versions() {
            for node in versions() {
                let expected = match (parse_semver(&gateway), parse_semver(&node)) {
                    (Some(g), Some(n)) => match (g.major, n.major) {
                        (1, 1) => g.minor == n.minor,
                        (2, 2) => modern_edges.contains(&(g.minor, n.minor)),
                        (3, 3) => true,
                        (2, 1) => g.minor == 0 && n.minor == 19,
                        (1, 2) => g.minor == 19 && n.minor == 0,
                        _ => false,
                    },
                    _ => false,
                };
                assert_eq!(is_compatible(&gateway, &node), expected, "{gateway:?} -> {node:?}");
            }
        }
        for (g, n) in [("2.1.0", "1.19.11"), ("1.19.11", "2.1.0")] { assert!(!is_compatible(g, n)); }
    }

    #[test]
    fn refused_pairs_name_the_older_side() {
        for gateway in versions() {
            for node in versions() {
                if is_compatible(&gateway, &node) { continue; }
                let message = describe_mismatch(&gateway, &node);
                match (parse_semver(&gateway), parse_semver(&node)) {
                    (_, None) => assert!(message.contains("노드를 업그레이드하세요")),
                    (None, _) => assert!(message.contains("게이트웨이 버전을 알 수 없습니다")),
                    (Some(g), Some(n)) => assert!(message.contains(if (g.major, g.minor) < (n.major, n.minor) { "게이트웨이를 올리세요" } else { "노드를 올리세요" }), "{gateway} -> {node}: {message}"),
                }
            }
        }
    }

    #[test]
    fn mismatch_text_is_the_ts_text() {
        assert_eq!(describe_mismatch("1.17.0", "1.16.4"), "노드 버전이 낮습니다 — 노드를 올리세요. (게이트웨이 1.17.0 / 노드 1.16.4)");
        assert_eq!(describe_mismatch("1.16.0", "1.17.2"), "게이트웨이 버전이 낮습니다 — 게이트웨이를 올리세요. (게이트웨이 1.16.0 / 노드 1.17.2)");
        assert_eq!(describe_mismatch("", "1.17.2"), "게이트웨이 버전을 알 수 없습니다. (노드 1.17.2)");
        assert_eq!(describe_mismatch("", "dev"), "이 노드에는 노드 기능이 없습니다. 노드를 업그레이드하세요. (게이트웨이 알 수 없음)");
        assert_eq!(describe_mismatch("1.2.3", "dev"), "이 노드에는 노드 기능이 없습니다. 노드를 업그레이드하세요. (게이트웨이 1.2.3)");
    }
}
