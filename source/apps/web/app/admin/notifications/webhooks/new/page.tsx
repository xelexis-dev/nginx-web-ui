import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { DEFAULT_EVENT_TYPES } from "@repo/presentation/webhook";
import { readPageApi } from "@/lib/api-page";
import { WebhookForm } from "../webhook-form";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

// 생성은 모달이 아니라 별도 페이지다(프로젝트 규칙). 수정은 /[id]/edit.
export default async function NewWebhookPage() {
  await readPageApi("/api/ui/v1/admin/notifications/webhooks");

  return (
    <section className={layout.page}>
      <PageHeader title={<>새 webhook</>} />

      <WebhookForm
        mode="create"
        defaults={{
          name: "",
          enabled: true,
          format: "generic",
          events: [...DEFAULT_EVENT_TYPES],
          bodyTemplate: "",
        }}
      />
    </section>
  );
}
