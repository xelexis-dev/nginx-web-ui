import { readPageApi } from "@/lib/api-page";
import type { MailRecipientRow, SmtpView } from "@repo/contracts";
import { RecipientListView } from "./recipient-list";
export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export default async function MailRecipientsPage() {
  const [recipients, smtp] = await Promise.all([
    readPageApi<MailRecipientRow[]>("/api/ui/v1/admin/notifications/mail"),
    readPageApi<SmtpView>("/api/ui/v1/admin/settings/smtp"),
  ]);
  const mailer = smtp.readiness;

  return <RecipientListView recipients={recipients} mailer={mailer} />;
}
