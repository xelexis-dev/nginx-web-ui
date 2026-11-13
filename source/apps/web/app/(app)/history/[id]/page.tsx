import { readPageView } from "@/lib/api-page";
import type { HistoryDetailView } from "@/lib/overview-views";
import { HistoryDetail } from "@/components/history/history-detail";
export const runtime = "nodejs";
export default async function HistoryDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  return <HistoryDetail data={await readPageView<HistoryDetailView>("/history/" + encodeURIComponent(id))} />;
}
