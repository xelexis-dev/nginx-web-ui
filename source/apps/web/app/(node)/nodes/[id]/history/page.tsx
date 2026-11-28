import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { HistoryList, NODE_HISTORY_LIST_INFO, type HistoryListVM, type HistoryRowVM } from "@/components/history/history-list";
import { loadNodePage } from "@/lib/node-page";

export const runtime = "nodejs";

/** Row returned by the existing node history list endpoint. */
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

  return <HistoryList data={loaded.data} basePath={basePath} info={NODE_HISTORY_LIST_INFO} />;
}
