import { FormStack, Banner } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import type { SmtpView } from "@repo/contracts";
import { DEFAULT_EVENT_TYPES } from "@repo/presentation/webhook";
import { readPageApi } from "@/lib/api-page";

import { MailRecipientForm } from "../recipient-form";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

// 생성은 모달이 아니라 별도 페이지다(프로젝트 규칙). 수정은 /[id]/edit.
export default async function NewMailRecipientPage() {
  const { readiness: mailer } = await readPageApi<SmtpView>(
    "/api/ui/v1/admin/settings/smtp",
  );

  return (
    <section className={layout.page}>
      <PageHeader title={<>새 수신처</>} />

      <FormStack>
        <Banner
          tone="info"
          title={<>인증서 알림을 받을 주소를 등록합니다.</>}
        />
        <MailRecipientForm
          mode="create"
          smtpReady={mailer.ok}
          smtpProblem={mailer.ok ? null : mailer.message}
          defaults={{
            label: "",
            address: "",
            enabled: true,
            events: [...DEFAULT_EVENT_TYPES],
          }}
        />
      </FormStack>
    </section>
  );
}
