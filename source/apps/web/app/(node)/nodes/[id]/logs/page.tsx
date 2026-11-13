import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { LogViewer } from "@/components/logs/log-viewer";
import { authorizeNodePage } from "@/lib/node-page";

// 노드의 실시간 로그. LogViewer(components/logs/log-viewer.tsx)가 클라이언트
// 쪽에서 스스로 EventSource를 열어 게이트웨이 중계 라우트
// (/api/nodes/[id]/logs/stream)를 부른다 — Rust nodes/relay.rs가 RBAC을
// 다시 본다. Metrics와 같은 구조다:
// 데이터를 이 화면이 서버에서 내려주는 게 아니라 클라이언트가 직접 열기
// 때문에 loadNodePage로 초기 데이터를 내려주지 않는다.
//
// 이 페이지의 역할은 (1) 노드가 없으면 404, (2) 게이트웨이 사용자 등급과
// 노드 토큰 등급을 min해 config:read가 안 되면 §9 카드를 보여주는 것 — 나머지
// 노드 화면(audit·history·metrics)과 같은 진입 판정이다.
export const runtime = "nodejs";

export default async function NodeLogsPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const path = `/nodes/${id}/logs`;
  const access = await authorizeNodePage({nodeId:id,action:"config:read",retryHref:path,listHref:path});
  if (!access.ok) return <section className={layout.page}><PageHeader title="로그" />{access.render}</section>;

  return (
    <section className={layout.page}>
      <PageHeader eyebrow={access.node.name} title={<>로그</>} info={<>access.log / error.log를 실시간으로 확인합니다.</>} />

      <div>
        <LogViewer streamPath={`/api/nodes/${id}/logs/stream`} />
      </div>
    </section>
  );
}
