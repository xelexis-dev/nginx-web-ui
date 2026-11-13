import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";

import type { MailRecipientRow, SmtpView } from "@repo/contracts";
import { readPageApi } from "@/lib/api-page";

import { MailRecipientForm } from "../../recipient-form";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export default async function EditMailRecipientPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const [recipient, smtp] = await Promise.all([
    readPageApi<MailRecipientRow>(
      `/api/ui/v1/admin/notifications/mail/${encodeURIComponent(id)}`,
    ),
    readPageApi<SmtpView>("/api/ui/v1/admin/settings/smtp"),
  ]);
  const mailer = smtp.readiness;

  return (
    <section className={layout.page}>
      <PageHeader
        title={<>수신처 수정</>}
        description={<>{recipient.address}</>}
      />

      <MailRecipientForm
        mode="edit"
        smtpReady={mailer.ok}
        smtpProblem={mailer.ok ? null : mailer.message}
        defaults={{
          id: recipient.id,
          label: recipient.label ?? "",
          address: recipient.address,
          enabled: recipient.enabled,
          events: (Array.isArray(recipient.events)
            ? recipient.events
            : []) as string[],
        }}
      />
    </section>
  );
}
