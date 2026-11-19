import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { readPageApi } from "@/lib/api-page";
import type { ProviderView } from "../../form-types";
import { ProviderForm } from "../../provider-form";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export default async function EditOAuthProviderPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const provider = await readPageApi<ProviderView & { callbackUrl: string }>(`/api/ui/v1/admin/oauth/${encodeURIComponent(id)}`);

  return (
    <section className={layout.page}>
      <PageHeader title={<>공급자 수정</>} description={<>{provider.name}</>} />

      <ProviderForm
        mode="edit"
        defaults={{
          id: provider.id,
          name: provider.name,
          issuer: provider.issuer,
          clientId: provider.clientId,
          scope: provider.scope,
          enabled: provider.enabled,
          sortOrder: provider.sortOrder,
        }}
        callbackUrl={provider.callbackUrl}
      />
    </section>
  );
}
