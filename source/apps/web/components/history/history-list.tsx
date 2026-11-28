import layout from "@/components/page-layout.module.css";
import {
  EmptyState,
  Surface,
  Table,
  Th,
  Td,
  Tr,
  StatusBadge,
  PageHeader,
} from "@repo/ui";
import type { RetentionCleanup } from "@repo/contracts";
import Link from "next/link";
import { HistoryTime, NodeHistoryLink } from "./history-time";
import styles from "./history.module.css";

export const NODE_HISTORY_LIST_INFO =
  "이 노드에 설정을 적용(apply)할 때마다 저장된 스냅샷 이력입니다. 최근 100건을 표시하며, 각 항목을 열면 스냅샷 내용과 롤백 버튼을 확인할 수 있습니다.";

export interface HistoryRowVM {
  id: string;
  /** UTC ISO string — `HistoryTime` formats it in the browser's timezone client-side. */
  iso: string;
  label: string | null;
  /** Omitted entirely (not just `null`) in node context — the node can't resolve `actorId` to an email. */
  actorEmail?: string | null;
  /** All three omitted together in node context — the node's list endpoint doesn't send `data`. */
  upstreamCount?: number;
  serverCount?: number;
  streamCount?: number;
}

export interface HistoryListVM {
  items: HistoryRowVM[];
  /** true: 사용자·Upstreams·Servers·Streams 열도 그린다(로컬). false: 시간·라벨만(노드). */
  showDetails: boolean;
  lastCleanup?: RetentionCleanup | null;
}

export interface HistoryListProps {
  data: HistoryListVM;
  title?: string;
  info: string;
  basePath?: string;
}

export function HistoryList({
  data,
  title = "적용 이력",
  info,
  basePath,
}: HistoryListProps) {
  const { items, showDetails, lastCleanup } = data;

  return (
    <section className={layout.page}>
      <PageHeader title={<>{title}</>} info={<>{info}</>} />

      <Surface flush>
        <Table density="comfortable" label="설정 적용 이력">
          <thead>
            <Tr>
              <Th>라벨</Th>
              <Th>시간</Th>
              {showDetails ? (
                <>
                  <Th>사용자</Th>
                  <Th align="end">Upstreams</Th>
                  <Th align="end">Servers</Th>
                  <Th align="end">Streams</Th>
                </>
              ) : null}
            </Tr>
          </thead>
          <tbody>
            {items.length === 0 ? (
              <Tr>
                <Td colSpan={showDetails ? 6 : 2}>
                  <EmptyState description="적용 이력이 없습니다." />
                </Td>
              </Tr>
            ) : null}
            {items.map((row, index) => (
              <Tr key={row.id} current={index === 0}>
                <Td cell="primary" wrap>
                  <div className={styles.label}>
                    {basePath ? (
                      showDetails ? (
                        <Link href={`${basePath}/${encodeURIComponent(row.id)}`}>{row.label ?? "—"}</Link>
                      ) : (
                        <NodeHistoryLink href={`${basePath}/${encodeURIComponent(row.id)}`} label={row.label} iso={row.iso} />
                      )
                    ) : (
                      <span>{row.label ?? "—"}</span>
                    )}
                    {index === 0 ? (
                      <StatusBadge
                        tone="info"
                        dot={false}
                        className={styles.current}
                      >
                        현재
                      </StatusBadge>
                    ) : null}
                  </div>
                </Td>
                <Td cell="meta" label="시간">
                  <HistoryTime iso={row.iso} />
                </Td>
                {showDetails ? (
                  <>
                    <Td cell="meta" label="사용자" wrap>
                      {row.actorEmail ?? "system"}
                    </Td>
                    <Td cell="meta" label="Upstreams" numeric>
                      {row.upstreamCount}
                    </Td>
                    <Td cell="meta" label="Servers" numeric>
                      {row.serverCount}
                    </Td>
                    <Td cell="meta" label="Streams" numeric>
                      {row.streamCount}
                    </Td>
                  </>
                ) : null}
              </Tr>
            ))}
          </tbody>
        </Table>
        {lastCleanup && (
          <p className={layout.listNote}>
            목록은 최근 50건까지 보여 줍니다. 그보다 오래된 이력 가운데 일부는 보존 기간이 지나 지워졌습니다. 마지막 정리{" "}
            <HistoryTime iso={lastCleanup.at} /> · {lastCleanup.deleted.toLocaleString("ko-KR")}건
          </p>
        )}
      </Surface>
    </section>
  );
}
