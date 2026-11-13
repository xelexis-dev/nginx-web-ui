import type { AuditListView, CertificateListView, DashboardView, HistoryView, NodesListView, ServerListView } from "@repo/contracts";
import { readPageView } from "./api-page";
import { readOptionalView } from "./optional-read";
import { toOverviewVM, type OverviewVM } from "./overview-data";

/**
 * 개요 화면의 읽기. 필수(`/`)는 `readPageView` 라 실패하면 페이지 오류·로그인 이동이 평소대로
 * 일어나고, 나머지는 나란히 읽되 각자 실패한다(`overview-data.ts` 머리 설명).
 */
export async function readOverview(): Promise<OverviewVM> {
  const [dashboard, servers, certificates, nodes, audit, history] = await Promise.all([
    readPageView<DashboardView>("/"),
    readOptionalView<ServerListView>("/reverse-proxy"),
    readOptionalView<CertificateListView>("/certificates"),
    readOptionalView<NodesListView>("/nodes"),
    readOptionalView<AuditListView>("/audit"),
    readOptionalView<HistoryView>("/history"),
  ]);
  return toOverviewVM({ dashboard, servers, certificates, nodes, audit, history });
}
