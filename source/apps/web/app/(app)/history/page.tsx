import { readPageView } from "@/lib/api-page";
import type { HistoryView } from "@/lib/overview-views";
import { HistoryList } from "@/components/history/history-list";

export const runtime = "nodejs";

export default async function HistoryPage() {
  const { data } = await readPageView<HistoryView>("/history");
  return <HistoryList data={data} basePath="/history" info="설정 적용(apply) 시점마다 저장된 스냅샷 이력을 확인합니다. 최근 50건을 표시하며, 각 항목을 열면 스냅샷 내용과 롤백 버튼을 확인할 수 있습니다." />;
}
