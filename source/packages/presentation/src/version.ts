/**
 * 게이트웨이와 노드의 호환 판정. 별도 apiVersion 을 두지 않고 **제품 semver 의
 * 같은 major(2 이상)에서 게이트웨이 minor ≥ 노드 minor**로 정한다.
 * 1.x는 major.minor 일치를 유지하고, 2.0 ↔ 1.19 예외만 허용한다.
 *
 * 이 규칙은 릴리스 규율을 하나 만든다: **patch 릴리스에서는 노드 API 계약을 바꾸지
 * 않는다.** 계약이 바뀌면 patch 가 아니라 minor 를 올린다.
 */

export interface Semver {
  major: number;
  minor: number;
  patch: number;
}

const SEMVER_RE = /^(\d+)\.(\d+)\.(\d+)/;

/** `1.15.3` / `2.0.0-rc.1` → 숫자 셋. 형식이 아니면 null. */
export function parseSemver(value: string): Semver | null {
  const m = SEMVER_RE.exec(value.trim());
  if (!m) return null;
  return { major: Number(m[1]), minor: Number(m[2]), patch: Number(m[3]) };
}

/** 불일치를 사람이 읽는 한 줄로. 어느 쪽을 올려야 하는지까지 말한다. */
export function describeMismatch(gateway: string, node: string): string {
  const g = parseSemver(gateway);
  const n = parseSemver(node);
  if (!n) {
    return `이 노드에는 노드 기능이 없습니다. 노드를 업그레이드하세요. (게이트웨이 ${gateway || "알 수 없음"})`;
  }
  if (!g) {
    return `게이트웨이 버전을 알 수 없습니다. (노드 ${node})`;
  }
  const nodeIsOlder = n.major < g.major || (n.major === g.major && n.minor < g.minor);
  return nodeIsOlder
    ? `노드 버전이 낮습니다 — 노드를 올리세요. (게이트웨이 ${gateway} / 노드 ${node})`
    : `게이트웨이 버전이 낮습니다 — 게이트웨이를 올리세요. (게이트웨이 ${gateway} / 노드 ${node})`;
}
