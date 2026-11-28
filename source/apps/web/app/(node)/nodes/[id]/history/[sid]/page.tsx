import type { NodeHistoryDetailData } from "@repo/contracts";
import { PageHeader } from "@repo/ui";
import { HistoryDetail } from "@/components/history/history-detail";
import layout from "@/components/page-layout.module.css";
import { loadNodePage } from "@/lib/node-page";

export const runtime = "nodejs";

export default async function NodeHistoryDetailPage({
  params,
}: {
  params: Promise<{ id: string; sid: string }>;
}) {
  const { id, sid } = await params;
  const segment = encodeURIComponent(sid);
  const listHref = `/nodes/${id}/history`;
  const href = `${listHref}/${segment}`;
  const loaded = await loadNodePage<NodeHistoryDetailData>({
    nodeId: id,
    action: "config:read",
    path: `/api/v1/history/${segment}`,
    retryHref: href,
    listHref,
    toViewModel: (raw) => raw as NodeHistoryDetailData,
  });

  if (!loaded.ok) {
    return <section className={layout.page}><PageHeader title="적용 이력" />{loaded.render}</section>;
  }

  return <HistoryDetail data={loaded.data} node={{ id, name: loaded.node.name, href }} />;
}
