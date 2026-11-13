import layout from "@/components/page-layout.module.css";
import Link from "next/link";
import { PageHeader, linkClassName } from "@repo/ui";
import { UpstreamForm } from "@/components/upstreams/upstream-form";
import { loadNodePage } from "@/lib/node-page";

// 노드의 upstream 수정 화면. 폼 초기값을 위해 `GET /api/v1/upstreams/:id` 를
// 부른다 — 읽기지만 **`config:write` 로 연다**: 쓰기 화면이라 그 등급이 없으면
// 폼을 띄우는 것 자체가 헛일이다(로컬 `edit/page.tsx` 의
// `requireRole("config:write")` 에 대응). 노드 쪽 GET 라우트는 `config:read`
// 만 요구하므로 이 판정은 게이트웨이가 화면을 가리기 위한 것이다(스펙 §8).
export const runtime = "nodejs";

interface NodeUpstreamDetailResponse {
  upstream: {
    id: string;
    name: string;
    servers: unknown;
    version?: string;
    resolveAtRuntime?: boolean;
  };
}

export default async function EditNodeUpstreamPage({
  params,
}: {
  params: Promise<{ id: string; upstreamId: string }>;
}) {
  const { id, upstreamId } = await params;
  const basePath = `/nodes/${id}/reverse-proxy/upstreams`;

  const loaded = await loadNodePage<{
    id: string;
    name: string;
    servers: string[];
    version?: string;
    resolveAtRuntime?: boolean;
  }>({
    nodeId: id,
    action: "config:write",
    path: `/api/v1/upstreams/${encodeURIComponent(upstreamId)}`,
    retryHref: `${basePath}/${upstreamId}/edit`,
    // `not_found` 면 그 항목이 이미 없다 — 수정 화면으로 되돌리지 않는다.
    listHref: basePath,
    toViewModel: (raw) => {
      const { upstream } = raw as NodeUpstreamDetailResponse;
      return {
        id: upstream.id,
        version: upstream.version,
        name: upstream.name,
        resolveAtRuntime: upstream.resolveAtRuntime,
        servers: Array.isArray(upstream.servers)
          ? (upstream.servers as string[])
          : [],
      };
    },
  });

  if (!loaded.ok)
    return (
      <section className={layout.page}>
        <PageHeader title="Upstream 수정" />
        {loaded.render}
      </section>
    );

  return (
    <section className={layout.page}>
      <PageHeader
        eyebrow={
          <Link
            href={`${basePath}/${upstreamId}`}
            className={linkClassName({ variant: "back" })}
          >
            {loaded.data.name}
          </Link>
        }
        title="Upstream 수정"
      />

      <UpstreamForm
        target={{ kind: "node", id, name: loaded.node.name }}
        nodeVersionFamily={/^1\.19\.\d+/.test(loaded.node.lastVersion ?? "") ? "1.19" : "2.0"}
        runtimeDns={loaded.runtimeDns}
        action={{ id: "node.updateNodeUpstream", args: [id] }}
        defaultValues={loaded.data}
        cancelHref={`${basePath}/${upstreamId}`}
      />
    </section>
  );
}
