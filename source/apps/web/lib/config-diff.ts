/** API diff presentation types and defensive remote response reader. */
import type { ConfigDiff, NodeFieldChange, NodeItemDiff } from "@repo/contracts";
export type { ConfigDiff } from "@repo/contracts";
export type FieldChange = NodeFieldChange;
export type ItemDiff = NodeItemDiff;
export type ItemStatus = NodeItemDiff["status"];
const STATUS_ORDER: Record<ItemStatus, number> = { skipped: 0, added: 1, changed: 2, removed: 3 };

export function readConfigDiff(diff: ConfigDiff): ConfigDiff {
  return {
    servers: readItemDiffs(diff.servers, "servers"),
    streams: readItemDiffs(diff.streams, "streams"),
    upstreams: readItemDiffs(diff.upstreams, "upstreams"),
    // 노드 화면은 이 값을 읽지 않는다(배지 숫자는 `pending.count` 다) — 그대로 나른다.
    hasChanges: diff.hasChanges,
  };
}

function readItemDiffs(items: ItemDiff[], group: string): ItemDiff[] {
  return items.map((item, i) => {
    const at = `${group}[${i}]`;
    if (!(item.status in STATUS_ORDER)) throw new TypeError(`${at}.status: 모르는 상태입니다.`);
    if (typeof item.label !== "string") throw new TypeError(`${at}.label: 문자열이 아닙니다.`);
    return {
      key: item.key,
      label: item.label,
      status: item.status,
      // 제외 사유는 화면에 그대로 뜬다 — 문자열이 아니면 나르지 않는다.
      ...(typeof item.reason === "string" ? { reason: item.reason } : {}),
      changes: item.changes.map((change, j) => {
        if (typeof change.field !== "string") throw new TypeError(`${at}.changes[${j}].field: 문자열이 아닙니다.`);
        return change;
      }),
    };
  });
}
