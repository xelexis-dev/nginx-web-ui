import type { AccountView } from "@repo/contracts";
import { readPageApi } from "@/lib/api-page";
import { passkeyListView, type PasskeyList } from "@/lib/passkey-api";
import { AccountOverview } from "./account-overview";

export const runtime = "nodejs";

export default async function AccountPage({ searchParams }: {
  searchParams: Promise<{ linked?: string; unlinked?: string; removed?: string; error?: string }>;
}) {
  const { linked, unlinked, removed, error } = await searchParams;
  const account = await readPageApi<AccountView>("/api/ui/v1/account");
  const passkeys = passkeyListView(await readPageApi<PasskeyList>("/api/ui/v1/account/passkeys"));
  const notice =
    linked === "1"
      ? ({ kind: "linked" } as const)
      : unlinked === "1"
        ? ({ kind: "unlinked" } as const)
        : error === "last-method"
          ? ({ kind: "last-method" } as const)
          : undefined;

  return <AccountOverview account={account} notice={notice} passkeys={passkeys} passkeyRemoved={removed === "1"} />;
}
