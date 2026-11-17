import type { AuditVia } from "@repo/contracts";
import { formatServerTime } from "@/lib/format-time";
import layout from "@/components/page-layout.module.css";
import {
  EmptyState,
  Icon,
  buttonClassName,
  Button,
  Surface,
  Table,
  Th,
  Td,
  Tr,
  PageHeader,
} from "@repo/ui";
import Link from "next/link";
import { ActionFilter } from "./action-filter";
import styles from "./audit.module.css";

/**
 * Body of the audit list screen. Shared by the local screen
 * (`app/(app)/audit/page.tsx`) and the node-context screen
 * (`app/(node)/nodes/[id]/audit/page.tsx`) — both read data from a different
 * source (local DB vs. `callNode`/`loadNodePage`) but render the same rows.
 * `components/` (not either route group) because both `(app)` and `(node)`
 * import it.
 */

export interface AuditRowVM {
  id: string;
  /** Already formatted — see `formatAuditTime` below. Never a raw Date/ISO string. */
  time: string;
  actorEmail: string | null;
  /** 사람이 아닌 주체(원격 게이트웨이)의 이름표. actorEmail 이 없을 때만 쓰인다. */
  actorLabel: string | null;
  via?: AuditVia | null;
  action: string;
  target: string | null;
}

export interface AuditListVM {
  items: AuditRowVM[];
  total: number;
  page: number;
  totalPages: number;
  actions: string[];
  /** `""` when no filter is applied — matches `<Select>`'s "전체 액션" option value. */
  actionFilter: string;
}

/**
 * Shared formatter so both call sites produce identical strings. The local
 * page has a `Date` straight from the DB; the node page must first turn the
 * node's ISO string into a `Date` (`new Date(iso)`) before calling this —
 * passing the ISO string through unformatted would show up as
 * `2026-08-28T05:19:20.792Z` on screen.
 */
export function formatAuditTime(value: Date): string {
  return formatServerTime(value);
}

export function formatAuditActor(row: Pick<AuditRowVM, "actorEmail" | "actorLabel" | "via">): string {
  const name = row.actorEmail ?? row.actorLabel ?? "system";
  return row.via?.kind === "token" ? `${name} · 토큰 ${row.via.name}` : name;
}

/** Pagination/filter links. `basePath` is `/audit` locally, `/nodes/<id>/audit` in node context. */
function buildHref(basePath: string, action: string, page: number): string {
  const params = new URLSearchParams();
  if (action) params.set("action", action);
  if (page > 1) params.set("page", String(page));
  const qs = params.toString();
  return qs ? `${basePath}?${qs}` : basePath;
}

export function AuditList({
  data,
  basePath,
}: {
  data: AuditListVM;
  basePath: string;
}) {
  const { items, total, page, totalPages, actions, actionFilter } = data;

  return (
    <section className={layout.page}>
      <PageHeader
        title={<>감사 로그</>}
        info={
          <>
            설정 변경, nginx 적용, 로그인/로그아웃 등 모든 감사 이벤트를
            확인합니다.
          </>
        }
      />

      <Surface
        flush
        toolbar={
          <div className={styles.toolbar}>
            <ActionFilter
              actions={actions}
              selected={actionFilter}
              basePath={basePath}
            />
            <span className={styles.totalLabel}>
              총 {total.toLocaleString("ko-KR")}건
            </span>
          </div>
        }
      >
        <Table label="감사 로그">
          <thead>
            <Tr>
              <Th>시간</Th>
              <Th>사용자</Th>
              <Th>액션</Th>
              <Th>대상</Th>
              <Th aria-label="상세">상세</Th>
            </Tr>
          </thead>
          <tbody>
            {items.length === 0 ? (
              <Tr>
                <Td colSpan={5}>
                  <EmptyState
                    description={
                      actionFilter
                        ? "해당 액션의 감사 로그가 없습니다."
                        : "감사 로그가 없습니다."
                    }
                  />
                </Td>
              </Tr>
            ) : null}
            {items.map((row) => (
              <Tr key={row.id}>
                <Td cell="primary">{row.time}</Td>
                <Td cell="meta" label="사용자">
                  {formatAuditActor(row)}
                </Td>
                <Td cell="meta" label="액션">
                  {row.action}
                </Td>
                <Td cell="meta" label="대상">
                  {row.target ?? "—"}
                </Td>
                <Td cell="action" className={styles.detailCell}>
                  <Link
                    href={`${basePath}/${row.id}`}
                    className={buttonClassName({
                      variant: "ghost",
                      size: "sm",
                    })}
                  >
                    보기
                  </Link>
                </Td>
              </Tr>
            ))}
          </tbody>
        </Table>
        <div className={styles.pagination}>
          {page > 1 ? (
            <Link
              href={buildHref(basePath, actionFilter, page - 1)}
              className={buttonClassName({ variant: "secondary", size: "sm" })}
            >
              <Icon name="arrowLeft" size={14} />
              이전
            </Link>
          ) : (
            <Button type="button" variant="secondary" size="sm" disabled>
              <Icon name="arrowLeft" size={14} />
              이전
            </Button>
          )}
          <span className={styles.pageStatus}>
            {page} / {totalPages}
          </span>
          {page < totalPages ? (
            <Link
              href={buildHref(basePath, actionFilter, page + 1)}
              className={buttonClassName({ variant: "secondary", size: "sm" })}
            >
              다음
              <Icon name="arrowRight" size={14} />
            </Link>
          ) : (
            <Button type="button" variant="secondary" size="sm" disabled>
              다음
              <Icon name="arrowRight" size={14} />
            </Button>
          )}
        </div>
      </Surface>
    </section>
  );
}
