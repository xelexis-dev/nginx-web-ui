import { FormStack, Banner } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import type { GeneralSettingsView } from "@repo/contracts";
import { readPageApi } from "@/lib/api-page";
import { BaseUrlForm } from "./base-url-form";
import { MetricsForm } from "./metrics-form";

export const runtime = "nodejs";
// Reads live DB state on every request; must not be prerendered at build time.
export const dynamic = "force-dynamic";

/**
 * 사이트 전체에 걸리는 설정. 지금은 앱 주소 하나뿐이지만, 채널(메일·webhook) 어느
 * 한쪽에 속하지 않는 값이라 여기가 제자리다 — 원래는 webhook 목록 아래 "공통 설정"에
 * 있었는데, 메일 본문의 링크도 같은 값을 쓰므로 webhook 의 것이 아니다.
 */
export default async function GeneralSettingsPage() {
  const settings = await readPageApi<GeneralSettingsView>(
    "/api/ui/v1/admin/settings/general",
  );
  const baseUrl = settings.publicBaseUrl;

  return (
    <section className={layout.page}>
      <PageHeader title={<>일반</>} />
      <FormStack>
        <Banner tone="info" title={<>사이트 전체에 적용되는 설정입니다.</>} />
        <BaseUrlForm defaultValue={baseUrl ?? ""} />
        <MetricsForm
          defaultRetentionDays={
            settings?.metricsRetentionDays != null
              ? String(settings.metricsRetentionDays)
              : ""
          }
          defaultExcludedHosts={settings?.metricsExcludedHosts ?? ""}
        />
      </FormStack>
    </section>
  );
}
