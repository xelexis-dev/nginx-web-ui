import { readPageView } from "@/lib/api-page";
import type { AuditDetailView } from "@/lib/overview-views";
import { AuditDetail, formatAuditJson } from "@/components/audit/audit-detail";
import { formatAuditTime } from "@/components/audit/audit-list";

export const runtime = "nodejs";

export default async function AuditDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const { log } = await readPageView<AuditDetailView>("/audit/" + encodeURIComponent(id));
  const data = { ...log, time: formatAuditTime(new Date(log.createdAt)), beforeJson: formatAuditJson(log.before), afterJson: formatAuditJson(log.after) };
  return <AuditDetail data={data} basePath="/audit" />;
}
