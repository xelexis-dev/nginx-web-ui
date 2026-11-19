import type { WebhookFormat } from "@repo/presentation/webhook";
import { readPageApi } from "@/lib/api-page";
import type { WebhookDetail } from "../form-types";
import { WebhookDetailActions } from "./detail-actions";

import { WebhookDetailView } from "../webhook-detail";
export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export default async function WebhookDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const target = await readPageApi<WebhookDetail>(
    `/api/ui/v1/admin/notifications/webhooks/${encodeURIComponent(id)}`,
  );
  const { maskedUrl, headerNames } = target;
  const events = (
    Array.isArray(target.events) ? target.events : []
  ) as string[];

  return (
    <WebhookDetailView
      target={target}
      maskedUrl={maskedUrl}
      headerNames={headerNames}
      events={events}
      detailActions={
        <WebhookDetailActions
          id={target.id}
          targetName={target.name}
          format={target.format as WebhookFormat}
          bodyTemplate={target.bodyTemplate}
        />
      }
    />
  );
}
