import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { MetricsView } from "@/components/metrics/metrics-view";
import { isRangeKey, type RangeKey } from "@/lib/metrics-view";
import { authorizeNodePage } from "@/lib/node-page";

// 노드의 접속 지표. 데이터는 이 화면이 아니라 클라이언트 쪽 MetricsView 가
// /api/nodes/[id]/metrics(중계 라우트)를 스스로 부른다 — 그 라우트가 RBAC 을
// 다시 본다(Rust nodes/relay.rs). 이 페이지의 역할은
// (1) 노드가 없으면 404, (2) 게이트웨이 사용자 등급과 노드 토큰 등급을 min 해
// config:read 가 안 되면 §9 카드를 보여주는 것 — 나머지 노드 화면(audit·
// history)과 같은 진입 판정을 여기서도 해서, 등급이 모자란 사용자가 빈 차트
// 대신 정돈된 카드를 보게 한다.
export const runtime = "nodejs";

export default async function NodeMetricsPage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>;
  searchParams: Promise<{ range?: string }>;
}) {
  const { id } = await params;
  const path = `/nodes/${id}/metrics`;
  const access = await authorizeNodePage({nodeId:id,action:"config:read",retryHref:path,listHref:path});
  if (!access.ok) return <section className={layout.page}><PageHeader title="접속 지표" />{access.render}</section>;

  const sp = await searchParams;
  const initialRange: RangeKey = isRangeKey(sp.range ?? null) ? (sp.range as RangeKey) : "24h";

  return (
    <section className={layout.page}>
      <PageHeader
        title={<>접속 지표</>}
        info={<>nginx 연결·요청 추이와 도메인별 요청 수입니다. 1분마다 수집합니다.</>}
      />

      <MetricsView initialRange={initialRange} dataPath={`/api/nodes/${id}/metrics`} />
    </section>
  );
}
