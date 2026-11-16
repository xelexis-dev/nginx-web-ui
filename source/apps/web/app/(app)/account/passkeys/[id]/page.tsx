import { readPageApi } from "@/lib/api-page";
import type { PasskeyDetail } from "@/lib/passkey-api";
import { PasskeyDetailView } from "../passkey-detail";
export const runtime = "nodejs";
export default async function PasskeyDetailPage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>;
  searchParams: Promise<{ added?: string; renamed?: string }>;
}) {
  const { id } = await params;
  const data = await readPageApi<PasskeyDetail>(
    `/api/ui/v1/account/passkeys/${encodeURIComponent(id)}`,
  );
  return <PasskeyDetailView data={data} query={await searchParams} />;
}
