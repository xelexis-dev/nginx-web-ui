import { readPageView } from "@/lib/api-page";
import type { AuditListView } from "@/lib/overview-views";
import { AuditList, formatAuditTime } from "@/components/audit/audit-list";

export const runtime = "nodejs";

export default async function AuditPage({ searchParams }: { searchParams: Promise<{ page?: string; action?: string }> }) {
  const view = await readPageView<AuditListView>("/audit", await searchParams);
  const data = { ...view, items: view.items.map(row => ({ ...row, time: formatAuditTime(new Date(row.createdAt)) })) };
  return <AuditList data={data} basePath="/audit" />;
}
