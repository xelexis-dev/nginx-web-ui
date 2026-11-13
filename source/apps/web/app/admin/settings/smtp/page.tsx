import { FormStack } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { Banner, PageHeader } from "@repo/ui";
import Link from "next/link";
import type { SmtpView } from "@repo/contracts";
import { readPageApi } from "@/lib/api-page";
import { SmtpForm } from "./smtp-form";

// The API authorizes the current administrator before returning safe settings.
export const runtime = "nodejs";
// Reads live API state on every request; must not be prerendered at build time.
export const dynamic = "force-dynamic";

export default async function SmtpPage() {
  const view = await readPageApi<SmtpView>("/api/ui/v1/admin/settings/smtp");

  return (
    <section className={layout.page}>
      <PageHeader
        title={<>SMTP 설정</>}
        description={
          <>
            받을 사람은{" "}
            <Link href="/admin/notifications/mail">알림 → 메일</Link>에서
            정합니다.
          </>
        }
      />
      {view.readiness.reason === "password-unreadable" ? (
        <Banner
          tone={"error"}
          title={<>{view.readiness.message}</>}
          role="alert"
        ></Banner>
      ) : null}
      <FormStack>
        <Banner tone="info" title={<>앱이 메일을 보낼 때 쓰는 서버입니다.</>} />
        <SmtpForm
          // 비밀번호는 내려보내지 않는다 — 저장 여부만 알려주고, 폼은 "유지/변경/삭제"만 다룬다.
          hasStoredPassword={view.hasStoredPassword}
          defaults={view.config}
        />
      </FormStack>
    </section>
  );
}
