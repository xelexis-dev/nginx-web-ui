import { readPageApi } from "@/lib/api-page";
import type { MailRecipientRow } from "../form-types";
import { MailRecipientDetailActions } from "./detail-actions";

import { RecipientDetailView } from "../recipient-detail";
export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export default async function MailRecipientDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const recipient = await readPageApi<MailRecipientRow>(
    `/api/ui/v1/admin/notifications/mail/${encodeURIComponent(id)}`,
  );

  const events = (
    Array.isArray(recipient.events) ? recipient.events : []
  ) as string[];

  return (
    <RecipientDetailView
      recipient={recipient}
      events={events}
      detailActions={
        <MailRecipientDetailActions
          id={recipient.id}
          address={recipient.address}
        />
      }
    />
  );
}
