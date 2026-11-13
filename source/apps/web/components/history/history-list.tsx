import layout from "@/components/page-layout.module.css";
import {
  Banner,
  Surface,
  Table,
  Th,
  Td,
  Tr,
  StatusBadge,
  PageHeader,
} from "@repo/ui";
import type { ReactNode } from "react";
import Link from "next/link";
import { HistoryTime } from "./history-time";
import styles from "./history.module.css";

/**
 * Body of the history list screen. Shared by the local screen
 * (`app/(app)/history/page.tsx`) and the node-context screen
 * (`app/(node)/nodes/[id]/history/page.tsx`) — both read data from a
 * different source (local DB vs. `callNode`/`loadNodePage`) but render the
 * same table. `components/` (not either route group) because both `(app)`
 * and `(node)` import it.
 *
 * Unlike audit-list.tsx (Task 3), the node response genuinely lacks columns
 * the local one has — no `data` (so no upstream/server/stream counts) and no
 * resolvable actor email (only a raw `actorId` the gateway can't turn into an
 * email). `showDetails` and `renderRowAction` let this stay one component
 * instead of two: the local page always passes `showDetails: true` and no
 * `renderRowAction` (rollback lives on its own detail page,
 * `(app)/history/[id]/page.tsx`, unchanged by this task); the node page
 * passes `showDetails: false` plus a `renderRowAction` that renders "노드에서
 * 열기" + a rollback button per row, since the node has no detail page for
 * rollback to live on (no `GET /api/v1/history/:id`).
 */

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
}

export interface HistoryListProps {
  data: HistoryListVM;
  title?: string;
  info: string;
  /** 목록 위 추가 안내 배너. 로컬은 안 넘긴다. */
  notice?: string;
  /** 있으면 시간 셀이 `${basePath}/${id}` 로 링크된다(로컬 상세 페이지). 노드는 상세가 없어 넘기지 않는다. */
  basePath?: string;
  /** 행마다 덧붙일 조작 UI(노드: "노드에서 열기" + 롤백). 로컬은 안 넘긴다. */
  renderRowAction?: (row: HistoryRowVM) => ReactNode;
}

export function HistoryList({
  data,
  title = "적용 이력",
  info,
  notice,
  basePath,
  renderRowAction,
}: HistoryListProps) {
  const { items, showDetails } = data;

  return (
    <section className={layout.page}>
      <PageHeader title={<>{title}</>} info={<>{info}</>} />

      {notice ? <Banner tone="neutral" title={notice} /> : null}

      <Surface flush>
        {items.length === 0 ? (
          <p className={styles.emptyCell}>적용 이력이 없습니다.</p>
        ) : (
          <Table label="설정 적용 이력">
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
                {renderRowAction ? <Th aria-label="조작" /> : null}
              </Tr>
            </thead>
            <tbody>
              {items.map((row, index) => (
                <Tr key={row.id} current={index === 0}>
                  <Td cell="primary" wrap>
                    <div className={styles.label}>
                      {basePath ? (
                        <Link href={`${basePath}/${row.id}`}>
                          {row.label ?? "—"}
                        </Link>
                      ) : (
                        <span>{row.label ?? "—"}</span>
                      )}
                      {index === 0 ? (
                        <StatusBadge tone="info" dot={false} className={styles.current}>
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
                  {renderRowAction ? (
                    <Td cell="action">{renderRowAction(row)}</Td>
                  ) : null}
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Surface>
    </section>
  );
}
