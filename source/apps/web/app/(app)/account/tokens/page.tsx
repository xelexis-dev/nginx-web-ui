import { readPageApi } from "@/lib/api-page";
import { readRevokedTokenName, tokenPresentation, type TokenApiList } from "@/lib/token-api";
import { AccountTokensView } from "./token-list";

export const runtime = "nodejs";

export default async function AccountTokensPage({
  searchParams,
}: {
  searchParams: Promise<{ revoked?: string | string[] }>;
}) {
  const { tokens } = await readPageApi<TokenApiList>("/api/ui/v1/account/tokens");
  const revokedName = await readRevokedTokenName("account", (await searchParams).revoked);
  return <AccountTokensView rows={tokens.map(tokenPresentation)} revokedName={revokedName} />;
}
