import { readPageApi } from "@/lib/api-page";
import type { WebhookRow } from "./form-types";
import { WebhookListView } from "./webhook-list";
export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export default async function WebhooksPage() {
  const targets = await readPageApi<WebhookRow[]>(
    "/api/ui/v1/admin/notifications/webhooks",
  );

  return <WebhookListView targets={targets} />;
}
