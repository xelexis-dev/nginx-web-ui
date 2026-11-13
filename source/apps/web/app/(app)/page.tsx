import { OperationsOverview } from "@/components/operations/overview";
import { readOverview } from "@/lib/overview-read";

export const runtime = "nodejs";

/**
 * 개요(Operations). 읽기는 `readOverview`(대시보드 뷰는 필수, 나머지 뷰는 보조 — 각자 실패), 그리기는
 * `OperationsOverview`(순수 표시). API 를 바꾸지 않고 이미 있는 뷰만 모은다.
 */
export default async function DashboardPage() {
  return <OperationsOverview vm={await readOverview()} />;
}
