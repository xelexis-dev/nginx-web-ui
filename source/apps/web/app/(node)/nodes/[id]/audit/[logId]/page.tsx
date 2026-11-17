import type { AuditVia } from "@repo/contracts";
import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { AuditDetail, formatAuditJson, type AuditDetailVM } from "@/components/audit/audit-detail";
import { formatAuditTime } from "@/components/audit/audit-list";
import { loadNodePage } from "@/lib/node-page";

// 노드의 감사 상세. `GET /api/v1/audit/:id` 하나로 그린다
// (app/api/v1/audit/[id]/route.ts 가 정본). 화면 본문은
// components/audit/audit-detail.tsx — 로컬 (app)/audit/[id]/page.tsx 와 같은
// 컴포넌트를 쓴다.
//
// **라우트 파라미터 이름에 주의.** `[id]` 는 노드, `[logId]` 는 이 화면이 보는
// 감사 행이다 — 둘 다 `[id]` 로 두면 Next 가 라우트를 거부하고, 이름을 헷갈리면
// 조용히 엉뚱한 걸 불러온다.
export const runtime = "nodejs";

/** `GET /api/v1/audit/:id` 의 `log`. `api/v1/audit/[id]/route.ts` 가 정본이다. */
interface NodeAuditLog {
  action: string;
  /** 노드는 ISO 문자열로 준다(`.toISOString()`) — 로컬의 `Date` 와 다르다. */
  createdAt: string;
  target: string | null;
  ip: string | null;
  actorEmail: string | null;
  actorLabel: string | null;
  via?: AuditVia | null;
  before: unknown;
  after: unknown;
}

interface NodeAuditDetailResponse {
  log: NodeAuditLog;
}

export default async function NodeAuditDetailPage({
  params,
}: {
  params: Promise<{ id: string; logId: string }>;
}) {
  const { id, logId } = await params;
  const basePath = `/nodes/${id}/audit`;

  const loaded = await loadNodePage<AuditDetailVM>({
    nodeId: id,
    action: "config:read",
    path: `/api/v1/audit/${logId}`,
    // 「다시 시도」는 지금 이 화면, 「목록으로 돌아가기」는 목록이다.
    // 예전에는 둘이 같은 prop 이라 사라진 행의 「목록으로」가 같은 상세로
    // 되돌아와 같은 카드를 다시 띄웠다 — `listHref` 가 그걸 고친다.
    retryHref: `${basePath}/${logId}`,
    listHref: basePath,
    toViewModel: (raw) => {
      const { log } = raw as NodeAuditDetailResponse;
      return {
        action: log.action,
        time: formatAuditTime(new Date(log.createdAt)),
        actorEmail: log.actorEmail,
        actorLabel: log.actorLabel,
        via: log.via,
        target: log.target,
        ip: log.ip,
        beforeJson: formatAuditJson(log.before),
        afterJson: formatAuditJson(log.after),
      };
    },
  });

  if (!loaded.ok) return <section className={layout.page}><PageHeader title="감사 로그" />{loaded.render}</section>;

  return <AuditDetail data={loaded.data} basePath={basePath} />;
}
