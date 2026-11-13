import { readPageApi } from "@/lib/api-page";
import type { ProviderView } from "../form-types";
import { ProviderDetailView, ProviderDetailActions } from "../provider-detail";
export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export default async function OAuthProviderDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const provider = await readPageApi<ProviderView & { callbackUrl: string }>(
    `/api/ui/v1/admin/oauth/${encodeURIComponent(id)}`,
  );

  return (
    <ProviderDetailView
      provider={provider}
      deleteForm={<ProviderDetailActions id={provider.id} />}
    />
  );
}
