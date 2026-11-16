import type { ConfigDiff, ItemDiff } from "./config-diff";
export type DiffGroup = "servers" | "upstreams" | "streams";
export const DIFF_GROUPS = ["servers", "upstreams", "streams"] as const;
export const GROUP_TITLES = {
  servers: "리버스 프록시",
  upstreams: "Upstream",
  streams: "Stream",
};
export function itemKind(item: ItemDiff) {
  return item.status === "skipped" || item.reason ? "skipped" : item.status;
}
export const DIFF_MARKERS = {
  skipped: { char: "◆", label: "적용에서 제외됨" },
  changed: { char: "~", label: "변경" },
  removed: { char: "−", label: "삭제" },
  added: { char: "+", label: "추가" },
} as const;
export function sortedItems(items: ItemDiff[]) {
  const order = { skipped: 0, changed: 1, removed: 2, added: 3 };
  return [...items].sort(
    (a, b) =>
      order[itemKind(a)] - order[itemKind(b)] || a.label.localeCompare(b.label),
  );
}
export function diffCounts(diff: ConfigDiff) {
  const counts = { added: 0, changed: 0, removed: 0, skipped: 0 };
  for (const group of DIFF_GROUPS)
    for (const item of diff[group]) counts[itemKind(item)]++;
  return counts;
}
export function serverLabel(label: string) {
  const at = label.lastIndexOf(":");
  return at < 0
    ? { name: label, port: null }
    : { name: label.slice(0, at), port: label.slice(at + 1) };
}
export function pendingIntro(
  diff: ConfigDiff,
  canApply = true,
  hasBaseline = true,
) {
  const counts = diffCounts(diff);
  const total = Object.values(counts).reduce((a, b) => a + b, 0);
  const prefix = hasBaseline
    ? ""
    : "마지막으로 적용된 설정을 확인하지 못해 저장된 설정을 모두 추가 항목으로 보여 줍니다. ";
  if (!canApply)
    return (
      prefix +
      "저장되었지만 아직 nginx에 적용되지 않은 변경입니다. 적용 권한이 있는 사용자가 적용하면 반영됩니다."
    );
  if (total > 0 && counts.skipped === total)
    return (
      prefix +
      "아래 항목은 적용에서 제외돼 있습니다. 표시된 이유를 고친 뒤 다시 적용하세요."
    );
  return (
    prefix +
    "저장된 변경으로 설정 파일을 만들고, nginx -t 검사를 통과한 경우에만 reload합니다." +
    (counts.skipped
      ? " 「적용에서 제외됨」 항목은 표시된 이유를 고치기 전에는 다시 적용해도 제외됩니다."
      : "")
  );
}
