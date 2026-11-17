import type { AuditVia } from "@repo/contracts";
import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { AuditList, formatAuditTime, type AuditListVM, type AuditRowVM } from "@/components/audit/audit-list";
import { loadNodePage } from "@/lib/node-page";

// 노드의 감사 목록. `GET /api/v1/audit` 하나로 그린다(app/api/v1/audit/route.ts
// 가 정본). 화면 본문은 components/audit/audit-list.tsx — 로컬
// (app)/audit/page.tsx 와 같은 컴포넌트를 쓴다.
export const runtime = "nodejs";

/** `GET /api/v1/audit` 의 한 행. `api/v1/audit/route.ts` 가 정본이다. */
interface NodeAuditRow {
  id: string;
  /** 노드는 ISO 문자열로 준다(`.toISOString()`) — 로컬의 `Date` 와 다르다. */
  createdAt: string;
  action: string;
  target: string | null;
  actorEmail: string | null;
  actorLabel: string | null;
  via?: AuditVia | null;
  ip: string | null;
}

/** `GET /api/v1/audit` 응답 모양. */
interface NodeAuditResponse {
  items: NodeAuditRow[];
  total: number;
  page: number;
  totalPages: number;
  actions: string[];
}

export default async function NodeAuditPage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>;
  searchParams: Promise<{ page?: string; action?: string }>;
}) {
  const { id } = await params;
  const sp = await searchParams;
  const actionFilter = sp.action?.trim() || "";
  const requestedPage = Number(sp.page);
  const page = Number.isInteger(requestedPage) && requestedPage > 0 ? requestedPage : 1;

  const query = new URLSearchParams();
  if (actionFilter) query.set("action", actionFilter);
  if (page > 1) query.set("page", String(page));
  const qs = query.toString();

  // `retryHref` = 지금 이 화면 자신의 주소(쿼리 포함) — `loadNodePage` 문서와
  // `NodeErrorCard` 의 「다시 시도」가 그 주소를 다시 여는 것과 같은 이유.
  const basePath = `/nodes/${id}/audit`;
  const selfHref = qs ? `${basePath}?${qs}` : basePath;

  const loaded = await loadNodePage<AuditListVM>({
    nodeId: id,
    action: "config:read",
    path: qs ? `/api/v1/audit?${qs}` : "/api/v1/audit",
    retryHref: selfHref,
    // 목록 화면이라 「다시 시도」와 「목록으로」가 같은 곳이다 — 같은 값을
    // 그대로 넘긴다(우연이 아니라 단언, `NodeErrorCardProps` 문서).
    listHref: selfHref,
    // 응답 모양이 어긋나면(items 가 배열이 아니거나 필드가 없으면) 여기서
    // 던지고, `loadNodePage` 가 그걸 `node_error` 카드로 접는다 — 방어 코드를
    // 두지 않는다(Global Constraints, node-page.tsx 문서).
    toViewModel: (raw) => {
      const res = raw as NodeAuditResponse;
      const items: AuditRowVM[] = res.items.map((row) => ({
        id: row.id,
        time: formatAuditTime(new Date(row.createdAt)),
        actorEmail: row.actorEmail,
        actorLabel: row.actorLabel,
        via: row.via,
        action: row.action,
        target: row.target,
      }));
      return {
        items,
        total: res.total,
        page: res.page,
        totalPages: res.totalPages,
        actions: res.actions,
        actionFilter,
      };
    },
  });

  if (!loaded.ok) return <section className={layout.page}><PageHeader title="감사 로그" />{loaded.render}</section>;

  return <AuditList data={loaded.data} basePath={basePath} />;
}
