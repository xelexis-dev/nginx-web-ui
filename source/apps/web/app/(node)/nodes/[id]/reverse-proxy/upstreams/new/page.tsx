import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { UpstreamForm } from "@/components/upstreams/upstream-form";
import { authorizeNodePage } from "@/lib/node-page";

// 노드에 upstream 을 새로 만드는 화면. **노드에서 읽어올 것이 없다** — 폼
// 하나뿐이라 `loadNodePage` 가 아니라 `authorizeNodePage` 로 접근만 판정한다
// (같은 §9 카드를 쓰되 왕복이 없다).
export const runtime = "nodejs";

export default async function NewNodeUpstreamPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const basePath = `/nodes/${id}/reverse-proxy/upstreams`;

  // 로컬 `new/page.tsx` 의 `requireRole("config:write")` 에 대응한다. 다른 점은
  // 노드 토큰 등급과 min 을 취한다는 것뿐이다(스펙 §8) — 사용자가 editor 라도
  // 노드 토큰이 viewer 면 여기서 forbidden 카드가 뜬다.
  const auth = await authorizeNodePage({
    nodeId: id,
    action: "config:write",
    retryHref: `${basePath}/new`,
    listHref: basePath,
  });
  if (!auth.ok)
    return (
      <section className={layout.page}>
        <PageHeader title="새 Upstream" />
        {auth.render}
      </section>
    );

  return (
    <section className={layout.page}>
      <PageHeader
        title="새 Upstream"
        info="새 upstream 그룹을 등록합니다."
      />

      {/* 서버 액션을 `nodeId` 로 묶어 폼에 넘긴다 — 폼이 보는 시그니처는
          `(prev, formData)` 그대로라 로컬 액션과 구분되지 않는다. */}
      <UpstreamForm
        target={{ kind: "node", id, name: auth.node.name }}
        runtimeDns={auth.runtimeDns}
        action={{ id: "node.createNodeUpstream", args: [id] }}
        cancelHref={basePath}
      />
    </section>
  );
}
