import layout from "@/components/page-layout.module.css";
import {
  Icon,
  Surface,
  Facts,
  CodePane,
  EmptyState,
  linkClassName,
  PageHeader,
} from "@repo/ui";
import type { HistoryDetailView } from "@/lib/overview-views";
import Link from "next/link";
import { ConfigDiffView } from "@/components/config-diff-view";
import { RollbackButton } from "@/app/(app)/history/rollback-button";
import { HistoryTime } from "@/components/history/history-time";
import styles from "@/app/(app)/history/history.module.css";

// Snapshot data and rollback permission come from the authenticated API.

function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

/** Detail page for one config snapshot. `notFound()` on unknown id. */
export function HistoryDetail({ data }: { data: HistoryDetailView }) {
  const {
    snapshot: row,
    upstreams: snapUpstreams,
    servers: snapServers,
    streams: snapStreams,
    diff,
    isFirst,
    canRollback,
    iso,
  } = data;
  // 롤백 확인 대화상자(client ConfirmDialog) 라벨용 문자열 — 라벨이 없을 때만 쓰이며,
  // 컴포넌트를 넘길 수 없어 서버에서 KST(폴백 시간대)로 포맷한 문자열을 쓴다.
  const kstLabel =
    new Intl.DateTimeFormat("ko-KR", {
      dateStyle: "medium",
      timeStyle: "medium",
      timeZone: "Asia/Seoul",
    }).format(new Date(iso)) + " KST";

  return (
    <section className={`${layout.page} ${styles.detailPage}`}>
      <PageHeader
        title={<>{row.label ?? <HistoryTime iso={iso} />}</>}
        actions={
          canRollback ? (
            <RollbackButton
              snapshotId={row.id}
              confirmLabel={row.label ?? kstLabel}
            />
          ) : null
        }
        eyebrow={
          <>
            {" "}
            <Link
              href="/history"
              className={linkClassName({ variant: "back" })}
            >
              <Icon name="arrowLeft" size={14} />
              목록으로
            </Link>{" "}
          </>
        }
      />

      <Surface as="div">
        <Facts
          items={[
            {
              term: <>시간</>,
              value: (
                <>
                  <HistoryTime iso={iso} />
                </>
              ),
            },
            { term: <>사용자</>, value: <>{row.actorEmail ?? "system"}</> },
            { term: <>라벨</>, value: <>{row.label ?? "—"}</> },
            { term: <>Upstreams</>, value: <>{snapUpstreams.length}건</> },
            { term: <>Servers</>, value: <>{snapServers.length}건</> },
            { term: <>Streams</>, value: <>{snapStreams.length}건</> },
          ]}
        />
      </Surface>

      <Surface title="변경 사항 (이전 스냅샷 대비)">
        {isFirst ? (
          <EmptyState description="최초 스냅샷입니다. 비교할 이전 스냅샷이 없습니다." />
        ) : !diff.hasChanges ? (
          <EmptyState description="이전 스냅샷과 동일합니다. (변경 없음)" />
        ) : (
          <ConfigDiffView diff={diff} variant="panel" />
        )}
      </Surface>

      <div className={styles.jsonSection}>
        <CodePane
          title="upstreams"
          code={formatJson(snapUpstreams)}
          copyLabel={false}
        />
        <CodePane
          title="servers"
          code={formatJson(snapServers)}
          copyLabel={false}
        />
        <CodePane
          title="streams"
          code={formatJson(snapStreams)}
          copyLabel={false}
        />
      </div>
    </section>
  );
}
