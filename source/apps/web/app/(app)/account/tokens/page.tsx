import { readPageApi } from "@/lib/api-page";
import { tokenPresentation, type TokenApiList } from "@/lib/token-api";
import { AccountTokensView } from "./token-list";

export const runtime = "nodejs";

export default async function AccountTokensPage() {
  const { tokens } = await readPageApi<TokenApiList>("/api/ui/v1/account/tokens");
  return <AccountTokensView rows={tokens.map(tokenPresentation)} />;
}
