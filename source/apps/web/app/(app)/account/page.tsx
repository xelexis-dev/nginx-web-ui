import type { AccountView } from "@repo/contracts";
import { readSession } from "@/lib/api-client";
import { readPageApi } from "@/lib/api-page";
import { passkeyListView, type PasskeyList } from "@/lib/passkey-api";
import { AccountOverview, type AccountNotice } from "./account-overview";

export const runtime = "nodejs";

export default async function AccountPage({
  searchParams,
}: {
  searchParams: Promise<{
    linked?: string;
    unlinked?: string;
    removed?: string;
    error?: string;
  }>;
}) {
  const { linked, unlinked, removed, error } = await searchParams;
  const account = await readPageApi<AccountView>("/api/ui/v1/account");
  const passkeys = passkeyListView(
    await readPageApi<PasskeyList>("/api/ui/v1/account/passkeys"),
  );
  const session = await readSession();
  const notice: { kind: AccountNotice } | undefined =
    removed === "1"
      ? ({ kind: "removed" } as const)
      : linked === "1"
        ? ({ kind: "linked" } as const)
        : unlinked === "1"
          ? ({ kind: "unlinked" } as const)
          : error === "last-method"
            ? ({ kind: "last-method" } as const)
            : error === "link-failed" || error === "already-linked"
              ? { kind: error }
              : undefined;

  return (
    <AccountOverview
      account={account}
      notice={notice}
      passkeys={passkeys}
      role={session?.user.role}
    />
  );
}
