import layout from "@/components/page-layout.module.css";
import { Banner, PageHeader } from "@repo/ui";
import type { WebhookFormat } from "@repo/presentation/webhook";
import { readPageApi } from "@/lib/api-page";
import type { WebhookDetail } from "../../form-types";
import { WebhookForm } from "../../webhook-form";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export default async function EditWebhookPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const target = await readPageApi<WebhookDetail>(
    `/api/ui/v1/admin/notifications/webhooks/${encodeURIComponent(id)}`,
  );
  const { maskedUrl, headerNames: storedHeaderNames } = target;

  return (
    <section className={layout.page}>
      <PageHeader title={<>webhook 수정</>} description={<>{target.name}</>} />

      {target.openError ? (
        <Banner
          tone={"error"}
          title={<>{target.openError}</>}
          role="alert"
        ></Banner>
      ) : null}

      <WebhookForm
        mode="edit"
        maskedUrl={maskedUrl}
        storedHeaderNames={storedHeaderNames}
        defaults={{
          id: target.id,
          name: target.name,
          enabled: target.enabled,
          format: target.format as WebhookFormat,
          events: (Array.isArray(target.events)
            ? target.events
            : []) as string[],
          bodyTemplate: target.bodyTemplate ?? "",
        }}
      />
    </section>
  );
}
