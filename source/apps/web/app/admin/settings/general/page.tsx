import { FormStack, Banner } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import type { GeneralSettingsView, RetentionSettingsView } from "@repo/contracts";
import { readPageApi } from "@/lib/api-page";
import { BaseUrlForm } from "./base-url-form";
import { MetricsForm } from "./metrics-form";
import { RetentionForm } from "./retention-form";

export const runtime = "nodejs";
// Reads live DB state on every request; must not be prerendered at build time.
export const dynamic = "force-dynamic";

// 앱 주소와 이 설치에만 적용되는 지표 및 기록 보존 설정.
export default async function GeneralSettingsPage() {
  const [settings, retention] = await Promise.all([
    readPageApi<GeneralSettingsView>("/api/ui/v1/admin/settings/general"),
    readPageApi<RetentionSettingsView>("/api/ui/v1/admin/settings/retention"),
  ]);
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
        <RetentionForm values={retention} />
      </FormStack>
    </section>
  );
}
