import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { StreamForm } from "@/components/streams/stream-form";
import { loadNodePage } from "@/lib/node-page";

// 노드의 stream 수정 화면. 폼 초기값을 위해 `GET /api/v1/streams/:id` 를 부른다 —
// 읽기지만 **`config:write` 로 연다**: 쓰기 화면이라 그 등급이 없으면 폼을 띄우는
// 것 자체가 헛일이다(로컬 `edit/page.tsx` 의 `requireRole("config:write")` 에
// 대응). 노드 쪽 GET 라우트는 `config:read` 만 요구하므로 이 판정은 게이트웨이가
// 화면을 가리기 위한 것이다(스펙 §8).
export const runtime = "nodejs";

interface NodeStreamDetailResponse {
  stream: { id: string; listen: number; protocol: "tcp" | "udp"; proxyPass: string; version?: string; resolveAtRuntime?: boolean };
}

export default async function EditNodeStreamPage({
  params,
}: {
  params: Promise<{ id: string; streamId: string }>;
}) {
  const { id, streamId } = await params;
  const basePath = `/nodes/${id}/stream`;

  const loaded = await loadNodePage<{ id: string; listen: number; protocol: "tcp" | "udp"; proxyPass: string; version?: string; resolveAtRuntime?: boolean }>({
    nodeId: id,
    action: "config:write",
    path: `/api/v1/streams/${encodeURIComponent(streamId)}`,
    retryHref: `${basePath}/${streamId}/edit`,
    // `not_found` 면 그 항목이 이미 없다 — 수정 화면으로 되돌리지 않는다.
    listHref: basePath,
    toViewModel: (raw) => {
      const { stream } = raw as NodeStreamDetailResponse;
      return { id: stream.id, version: stream.version, listen: stream.listen, protocol: stream.protocol, proxyPass: stream.proxyPass, resolveAtRuntime: stream.resolveAtRuntime };
    },
  });

  if (!loaded.ok) return loaded.render;

  return (
    <section className={layout.page}>
      <PageHeader title={<>Stream 수정</>} />

      <StreamForm target={{ kind: "node", id, name: loaded.node.name }}
        nodeVersionFamily={/^1\.19\.\d+/.test(loaded.node.lastVersion ?? "") ? "1.19" : "2.0"}
        runtimeDns={loaded.runtimeDns}
        action={{ id: "node.updateNodeStream", args: [id] }}
        defaultValues={loaded.data}
        cancelHref={`${basePath}/${streamId}`}
      />
    </section>
  );
}
