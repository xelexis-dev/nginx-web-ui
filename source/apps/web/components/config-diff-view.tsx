import { Icon, StatusBadge } from "@repo/ui";
import type { ConfigDiff, ItemDiff } from "@/lib/config-diff";
import styles from "./config-diff-view.module.css";

/** 변경 행 안의 단일 값 표시(짧게). */
function formatValue(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

const MARKER: Record<
  ItemDiff["status"],
  { char: string; className: string | undefined }
> = {
  added: { char: "+", className: styles.markerAdded },
  removed: { char: "−", className: styles.markerRemoved },
  changed: { char: "~", className: styles.markerChanged },
  // 적용이 거부한 행. 「올라갈 차례를 기다리는 것」과 같은 모양으로 그리면 안 된다.
  skipped: { char: "!", className: styles.markerSkipped },
};

/** 한 리소스 그룹(서버 / 스트림 / 업스트림). */
function DiffGroup({ title, items }: { title: string; items: ItemDiff[] }) {
  return (
    <div>
      <p className={styles.diffGroupTitle}>{title}</p>
      {items.length === 0 ? (
        <p className={styles.diffNone}>변경 없음</p>
      ) : (
        <ul className={styles.diffList}>
          {items.map((item) => (
            <li key={item.key} className={styles.diffItem}>
              <span className={styles.diffHead}>
                <span
                  className={`${styles.diffMarker} ${MARKER[item.status].className}`}
                >
                  {MARKER[item.status].char}
                </span>
                <span className={styles.diffLabel}>{item.label}</span>
                {item.status === "skipped" ? (
                  <StatusBadge
                    tone="error"
                    variant="outline"
                    size="sm"
                    dot={false}
                  >
                    적용에서 제외됨
                  </StatusBadge>
                ) : null}
              </span>
              {item.status === "skipped" && item.reason ? (
                <p className={styles.skippedReason}>{item.reason}</p>
              ) : null}
              {item.changes.length > 0 ? (
                <ul className={styles.fieldList}>
                  {item.changes.map((change) => (
                    <li key={change.field} className={styles.fieldChange}>
                      <span className={styles.fieldName}>{change.field}</span>
                      <span className={styles.fieldBefore}>
                        <span aria-hidden>− </span>
                        {formatValue(change.before)}
                      </span>
                      <span className={styles.fieldArrow}><Icon name="arrowRight" size={14} /></span>
                      <span className={styles.fieldAfter}>
                        <span aria-hidden>+ </span>
                        {formatValue(change.after)}
                      </span>
                    </li>
                  ))}
                </ul>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * ConfigDiff를 세 그룹으로 그리는 표시 전용 컴포넌트. 스냅샷 상세(/history/[id])와
 * 미적용 모달이 공유한다 — 같은 데이터를 두 곳에서 다르게 그리지 않도록.
 */
export function ConfigDiffView({
  diff,
  variant = "inline",
}: {
  diff: ConfigDiff;
  variant?: "inline" | "panel";
}) {
  return (
    <div
      className={[
        styles.diffGroups,
        variant === "panel" ? styles.panel : styles.inline,
      ].join(" ")}
      data-variant={variant}
    >
      <DiffGroup title="서버" items={diff.servers} />
      <DiffGroup title="스트림" items={diff.streams} />
      <DiffGroup title="업스트림" items={diff.upstreams} />
    </div>
  );
}
