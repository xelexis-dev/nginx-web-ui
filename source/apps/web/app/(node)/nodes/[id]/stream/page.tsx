import { can } from "@/lib/permissions";
import { readConfigDiff, type ConfigDiff } from "@/lib/config-diff";
import { StreamList, type StreamRowVM } from "@/components/streams/stream-list";
import { loadNodePageAll } from "@/lib/node-page";

// 로컬과 같은 목록 본문을 사용한다. 설정 적용은 노드 공통 헤더가 담당한다.
// 기존 페이지 응답의 pending 검증은 유지하되 목록에는 적용 액션을 넘기지 않는다.
export const runtime = "nodejs";

/** `GET /api/v1/streams` 의 한 행. 노드는 `nginx_streams` 테이블 행을 그대로 준다. */
interface NodeStreamRow {
  id: string;
  listen: number;
  protocol: "tcp" | "udp";
  proxyPass: string;
  enabled: boolean;
}

interface NodeStreamsResponse {
  streams: NodeStreamRow[];
}

/** `GET /api/v1/pending` 의 응답(`lib/pending.ts` 의 `PendingStatus` 그대로). */
interface NodePendingResponse {
  count: number;
  diff: ConfigDiff;
}

export default async function NodeStreamsPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const basePath = `/nodes/${id}/stream`;

  const loaded = await loadNodePageAll<"streams" | "pending", { items: StreamRowVM[]; pendingCount: number; pendingDiff: ConfigDiff }>({
    nodeId: id,
    action: "config:read",
    paths: { streams: "/api/v1/streams", pending: "/api/v1/pending" },
    // 실패 카드가 「무엇을」 못 읽었는지 말하도록 경로마다 이름을 준다 —
    // 카드 하나가 조회 둘을 대표하기 때문이다(`LoadNodePageAllArgs.subjects`).
    subjects: { streams: "Stream 목록을", pending: "미적용 변경을" },
    retryHref: basePath,
    // 목록 화면이라 둘이 같은 곳이다 — 같은 값을 그대로 넘긴다(우연이 아니라
    // 단언, `NodeErrorCardProps` 문서).
    listHref: basePath,
    // 응답 모양이 어긋나면 여기서 던지고 `loadNodePageAll` 이 `node_error` 카드로
    // 접는다 — 방어 코드를 두지 않는다(Global Constraints).
    toViewModel: (raw) => {
      const { streams } = raw.streams as NodeStreamsResponse;
      const pending = raw.pending as NodePendingResponse;
      return {
        items: streams.map(
          (s): StreamRowVM => ({
            id: s.id,
            listen: s.listen,
            protocol: s.protocol,
            proxyPass: s.proxyPass,
            enabled: s.enabled,
          }),
        ),
        // **노드가 준 값이다.** 게이트웨이의 `getPendingStatus()` 를 부르지 마라.
        pendingCount: pending.count,
        // 리버스 프록시 목록의 같은 자리와 **같은 함수**로 읽는다 — `.length`
        // 만으로는 안 닫힌다(문자열도 통과한다). `readConfigDiff` 문서 참고.
        pendingDiff: readConfigDiff(pending.diff),
      };
    },
  });

  if (!loaded.ok) return loaded.render;

  // 「+ 새 Stream」을 보여줄지는 목록을 읽는 데 필요한 `config:read` 와
  // 별개로 판정한다. **`effectiveRole` 을 손으로 다시 만들지 않는다** —
  // `resolveNode` 가 이미 min(사용자, 토큰)을 계산해 실어 준다(`NodePageData`
  // 문서). 재조립하면 §8 에 조건이 하나 늘 때 버튼은 그려지는데 액션은 막고,
  // 그 틈을 타입이 안 잡는다.
  const canWrite = can(loaded.effectiveRole, "config:write");

  return (
    <StreamList
      data={{ items: loaded.data.items, canWrite }}
      basePath={basePath}
    />
  );
}
