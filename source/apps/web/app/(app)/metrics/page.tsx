import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { MetricsView } from "@/components/metrics/metrics-view";
import { readPageView } from "@/lib/api-page";
import type { RangeKey } from "@/lib/metrics-view";

// 접속 지표 차트. 데이터는 /api/metrics 가 내려주고 거기서도 RBAC 을 다시 본다
// (클라이언트 fetch 는 서버 컴포넌트의 검사를 물려받지 않는다).
//
// 화면 본문(MetricsView)은 components/metrics/metrics-view.tsx — 노드 맥락
// 화면(app/(node)/nodes/[id]/metrics/page.tsx)과 같은 컴포넌트를 쓴다. 이
// 페이지는 기본 데이터 경로(`/api/metrics`)를 그대로 쓰므로 손댈 것이 초기
// range 계산뿐이다.
export const runtime = "nodejs";

export default async function MetricsPage({
  searchParams,
}: {
  searchParams: Promise<{ range?: string }>;
}) {
  const {initialRange} = await readPageView<{initialRange:RangeKey}>("/metrics",await searchParams);

  return (
    <section className={layout.page}>
      <PageHeader
        title={<>접속 지표</>}
        info={<>nginx 연결·요청 추이와 도메인별 요청 수입니다. 1분마다 수집합니다.</>}
      />

      <MetricsView initialRange={initialRange} dataPath="/api/ui/v1/metrics" />
    </section>
  );
}
