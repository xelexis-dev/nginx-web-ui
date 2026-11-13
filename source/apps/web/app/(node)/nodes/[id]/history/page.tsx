import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { can } from "@/lib/permissions";
import { HistoryList, type HistoryListVM, type HistoryRowVM } from "@/components/history/history-list";
import { loadNodePage } from "@/lib/node-page";
import { NodeHistoryActions } from "./rollback-button";

// 노드의 히스토리 목록. `GET /api/v1/history` 하나로 그린다
// (app/api/v1/history/route.ts 가 정본). 화면 본문은
// components/history/history-list.tsx — 로컬 (app)/history/page.tsx 와 같은
// 컴포넌트를 쓴다.
//
// **상세 화면은 없다.** `GET /api/v1/history/:id` 가 노드에 없다(patch 릴리스라
// 계약을 유지한다). 그래서 이 목록은 로컬과 달리 개수·작성자
// 열을 그리지 않고, 대신 각 행에 "노드에서 열기"(노드 자신의 UI로 새 탭) 와
// 롤백 버튼을 바로 둔다.
export const runtime = "nodejs";

/** `GET /api/v1/history` 의 한 행. `api/v1/history/route.ts:14-19` 가 정본이다. */
interface NodeSnapshotRow {
  id: string;
  /** 노드는 ISO 문자열로 준다(`.toISOString()`) — `HistoryTime` 이 클라이언트에서 그대로 포맷한다. */
  createdAt: string;
  label: string | null;
  /** 게이트웨이가 풀 수 없는 노드 쪽 사용자 id. 화면에 쓰지 않는다. */
  actorId: string | null;
}

interface NodeHistoryResponse {
  snapshots: NodeSnapshotRow[];
}

export default async function NodeHistoryPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const basePath = `/nodes/${id}/history`;

  const loaded = await loadNodePage<HistoryListVM>({
    nodeId: id,
    action: "config:read",
    path: "/api/v1/history",
    retryHref: basePath,
    // 목록 화면이라 둘이 같은 곳이다 — 같은 값을 그대로 넘긴다.
    listHref: basePath,
    toViewModel: (raw) => {
      const res = raw as NodeHistoryResponse;
      const items: HistoryRowVM[] = res.snapshots.map((row) => ({
        id: row.id,
        iso: row.createdAt,
        label: row.label,
      }));
      return { items, showDetails: false };
    },
  });

  if (!loaded.ok) return <section className={layout.page}><PageHeader title="적용 이력" />{loaded.render}</section>;

  // 롤백 버튼을 보여줄지는 목록을 읽는 데 필요한 `config:read` 와 별개로
  // 판정한다. `resolveNode(id, "nginx:reload")` 를 여기서 또 부르면 안 된다 —
  // `requireRole` 이 그 등급 없는 사용자(viewer)를 통째로 /login 으로 보내버려,
  // 목록조차 못 보게 된다(`lib/guard.ts` 의 `requireRole` 문서). 대신
  // `resolveNode` 가 이미 계산해 `loadNodePage` 가 실어 준 `effectiveRole`
  // (= min(사용자, 토큰), 스펙 §8)을 그대로 본다 — 손으로 재조립하면 §8 이
  // 바뀌는 날 버튼과 액션의 판정이 갈린다(`NodePageData` 문서).
  const canRollback = can(loaded.effectiveRole, "nginx:reload");

  return (
    <HistoryList
      data={loaded.data}
      info="이 노드에 적용된 설정 스냅샷 이력입니다. 게이트웨이는 스냅샷 내용을 읽을 수 없어 개수·작성자 열은 표시하지 않습니다."
      notice="스냅샷 내용은 노드에서만 볼 수 있습니다."
      renderRowAction={(row) => (
        <NodeHistoryActions
          nodeUrl={loaded.node.baseUrl}
          canRollback={canRollback}
          nodeId={id}
          nodeName={loaded.node.name}
          snapshotId={row.id}
          label={row.label}
          iso={row.iso}
          retryHref={basePath}
        />
      )}
    />
  );
}
