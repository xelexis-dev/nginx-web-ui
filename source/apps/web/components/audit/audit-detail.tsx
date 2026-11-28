import type { AuditVia } from "@repo/contracts";
import { formatAuditActor } from "./audit-list";
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
import Link from "next/link";

/**
 * Body of the audit detail screen. Shared by the local screen
 * (`app/(app)/audit/[id]/page.tsx`) and the node-context screen
 * (`app/(node)/nodes/[id]/audit/[logId]/page.tsx`) — see audit-list.tsx for
 * why this lives in `components/` instead of either route group.
 */

export interface AuditDetailVM {
  action: string;
  /** Already formatted — see `formatAuditTime` in audit-list.tsx. Never a raw Date/ISO string. */
  time: string;
  actorEmail: string | null;
  actorLabel: string | null;
  via?: AuditVia | null;
  target: string | null;
  ip: string | null;
  /**
   * Already pretty-printed (or `null` for absent/unprintable). Formatting
   * happens where the VM is built — locally in the page, or inside
   * `toViewModel` for the node page — not here, so a malformed `before`/
   * `after` from the node throws where `loadNodePage` catches it, not mid-render.
   */
  beforeJson: string | null;
  afterJson: string | null;
}

const MISSING_BEFORE = "이전 값이 없습니다.";
const MISSING_AFTER = "이후 값이 없습니다.";

/** Pretty-prints a `before`/`after` value for display. */
export function formatAuditJson(value: unknown): string | null {
  if (value === null || value === undefined) return null;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export function AuditDetail({
  data,
  basePath,
}: {
  data: AuditDetailVM;
  basePath: string;
}) {
  const {
    action,
    time,
    target,
    ip,
    beforeJson,
    afterJson,
  } = data;

  return (
    <section className={layout.page}>
      <PageHeader
        title={<>{action}</>}
        eyebrow={
          <>
            {" "}
            <Link
              href={basePath}
              className={linkClassName({ variant: "back" })}
            >
              <Icon name="arrowLeft" size={14} />
              감사 로그 목록
            </Link>{" "}
          </>
        }
      />

      <Surface as="div">
        <Facts
          items={[
            { term: <>시간</>, value: <>{time}</> },
            {
              term: <>사용자</>,
              value: <>{formatAuditActor(data)}</>,
            },
            { term: <>액션</>, value: <>{action}</> },
            { term: <>대상</>, value: <>{target ?? "—"}</> },
            { term: <>IP</>, value: <>{ip ?? "—"}</> },
          ]}
        />
      </Surface>

      <div className={layout.page}>
        {beforeJson === null ? (
          <Surface title="이전">
            <EmptyState description={MISSING_BEFORE} />
          </Surface>
        ) : (
          <CodePane title="이전" code={beforeJson} copyLabel={false} />
        )}
        {afterJson === null ? (
          <Surface title="이후">
            <EmptyState description={MISSING_AFTER} />
          </Surface>
        ) : (
          <CodePane title="이후" code={afterJson} copyLabel={false} />
        )}
      </div>
    </section>
  );
}
