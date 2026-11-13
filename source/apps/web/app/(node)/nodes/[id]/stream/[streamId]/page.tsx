import { configPreview, type ConfigPreview } from "@/lib/config-preview";
import { can } from "@/lib/permissions";
import { StreamDetail, type StreamDetailVM } from "@/components/streams/stream-detail";
import { loadNodePage } from "@/lib/node-page";

// 노드의 stream 상세. `GET /api/v1/streams/:id` 하나로 그린다
// (app/api/v1/streams/[id]/route.ts 가 정본). 화면 본문은
// components/streams/stream-detail.tsx — 로컬 상세와 같은 컴포넌트다.
//
// **라우트 파라미터 이름에 주의.** `[id]` 는 노드, `[streamId]` 는 이 화면이 보는
// stream 이다 — 둘 다 `[id]` 로 두면 Next 가 라우트를 거부하고, 이름을 헷갈리면
// 조용히 엉뚱한 걸 불러온다(노드 upstream·감사 상세와 같은 관용).
//
// 로컬은 행이 없으면 `notFound()` 지만 여기서는 노드가 404 를 내고
// `NodeErrorCard` 의 `not_found` 카드가 뜬다 — 스펙 §9 가 정한 모양이다.
export const runtime = "nodejs";

/** `GET /api/v1/streams/:id` 의 `stream`. */
interface NodeStreamDetailResponse {
  preview?: ConfigPreview;
  stream: { id: string; listen: number; protocol: "tcp" | "udp"; proxyPass: string; enabled: boolean; resolveAtRuntime?: boolean };
}

export default async function NodeStreamDetailPage({
  params,
}: {
  params: Promise<{ id: string; streamId: string }>;
}) {
  const { id, streamId } = await params;
  const basePath = `/nodes/${id}/stream`;

  const loaded = await loadNodePage<StreamDetailVM>({
    nodeId: id,
    action: "config:read",
    path: `/api/v1/streams/${encodeURIComponent(streamId)}`,
    // 「다시 시도」는 이 화면, 「목록으로 돌아가기」는 목록이다. 이 항목이
    // 사라져서 카드가 뜬 것이므로 되돌려 보내면 같은 카드가 다시 뜬다.
    retryHref: `${basePath}/${streamId}`,
    listHref: basePath,
    toViewModel: (raw) => {
      const { stream, preview } = raw as NodeStreamDetailResponse;
      return {
        id: stream.id,
        preview: configPreview(preview),
        listen: stream.listen,
        protocol: stream.protocol,
        proxyPass: stream.proxyPass,
        resolveAtRuntime: stream.resolveAtRuntime,
        enabled: stream.enabled,
      };
    },
  });

  if (!loaded.ok) return loaded.render;

  // 수정/토글/삭제를 보여줄지는 읽기 권한과 별개다. `resolveNode` 가 계산한
  // `effectiveRole`(= min(사용자, 토큰), 스펙 §8)을 그대로 쓴다 — 목록 화면과
  // 같은 이유로 손으로 재조립하지 않는다.
  const canWrite = can(loaded.effectiveRole, "config:write");

  return (
    <StreamDetail
      runtimeDns={loaded.runtimeDns}
      data={loaded.data}
      basePath={basePath}
      canWrite={canWrite}
      toggleAction={{ id: "node.toggleNodeStream", args: [id] }}
      deleteAction={{ id: "node.deleteNodeStream", args: [id] }}
    />
  );
}
