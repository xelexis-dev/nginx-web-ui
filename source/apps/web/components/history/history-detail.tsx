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
import type { NodeHistoryDetailData } from "@repo/contracts";
import type { HistoryDetailView } from "@/lib/overview-views";
import Link from "next/link";
import { ConfigDiffView } from "@/components/config-diff-view";
import { RollbackButton } from "@/app/(app)/history/rollback-button";
import { NodeRollbackButton } from "@/app/(node)/nodes/[id]/history/rollback-button";
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

type HistoryDetailProps =
  | { data: HistoryDetailView; node?: never }
  | { data: NodeHistoryDetailData; node: { id: string; name: string; href: string } };

const previousMessages: Partial<Record<NodeHistoryDetailData["comparison"], string>> = {
  previous_deleted: "이전 스냅샷은 노드에서 지워졌습니다. 비교할 수 없습니다.",
  outside_recent: "이 스냅샷 앞의 이력은 최근 100건 밖이라 확인할 수 없습니다. 비교할 수 없습니다.",
};

export function HistoryDetail({ data, node }: HistoryDetailProps) {
  const {
    snapshot: row,
    upstreams: snapUpstreams,
    servers: snapServers,
    streams: snapStreams,
    diff,
    canRollback,
    iso,
  } = data;
  const listHref = node ? `/nodes/${node.id}/history` : "/history";
  const first = "comparison" in data ? data.comparison === "first" : data.isFirst;
  const missingPrevious = "comparison" in data
    ? previousMessages[data.comparison]
    : data.previousPruned ? "이전 스냅샷은 보존 기간이 지나 지워졌습니다. 비교할 수 없습니다." : undefined;
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
          canRollback && (node ? (
            <NodeRollbackButton
              nodeId={node.id}
              nodeName={node.name}
              snapshotId={row.id}
              label={row.label}
              iso={iso}
              retryHref={node.href}
              listHref={listHref}
            />
          ) : (
            <RollbackButton snapshotId={row.id} confirmLabel={row.label ?? kstLabel} />
          ))
        }
        eyebrow={
          <>
            {" "}
            <Link
              href={listHref}
              className={linkClassName({ variant: "back" })}
            >
              <Icon name="arrowLeft" size={14} />
              적용 이력 목록
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
            ...(!node ? [{ term: <>사용자</>, value: <>{data.snapshot.actorEmail ?? "system"}</> }] : []),
            { term: <>라벨</>, value: <>{row.label ?? "—"}</> },
            { term: <>Upstreams</>, value: <>{snapUpstreams.length}건</> },
            { term: <>Servers</>, value: <>{snapServers.length}건</> },
            { term: <>Streams</>, value: <>{snapStreams.length}건</> },
          ]}
        />
      </Surface>

      <Surface title="변경 사항 (이전 스냅샷 대비)">
        {missingPrevious ? (
          <EmptyState description={missingPrevious} />
        ) : first ? (
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
