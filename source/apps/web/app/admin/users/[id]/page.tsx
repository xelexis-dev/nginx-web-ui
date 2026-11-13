import { readPageApi } from "@/lib/api-page";
import { readAdminSession, type AdminUserView } from "@/lib/admin-api";
import { UserDetailActions } from "./detail-actions";

import { AdminTwoFactor } from "../admin-two-factor";
import { AdminPasskeys } from "@/components/passkeys/admin-passkeys";
import { passkeyListView, type PasskeyList } from "@/lib/passkey-api";
import { UserDetailView } from "../user-detail";
export const runtime = "nodejs";
export default async function UserDetailPage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>;
  searchParams?: Promise<{ passkeysRemoved?: string; twoFactorReset?: string }>;
}) {
  const { id } = await params;
  const query = await searchParams;
  const session = await readAdminSession();

  const user = await readPageApi<AdminUserView>(
    `/api/ui/v1/admin/users/${encodeURIComponent(id)}`,
  );

  const passkeys = passkeyListView(
    await readPageApi<PasskeyList>(
      `/api/ui/v1/admin/users/${encodeURIComponent(id)}/passkeys`,
    ),
  );
  const isSelf = user.id === session.user.id;

  return (
    <UserDetailView
      user={user}
      isSelf={isSelf}
      twoFactor={
        <AdminTwoFactor
          user={user}
          isSelf={isSelf}
          canReset={passkeys.canRemove}
          passkeyCount={passkeys.passkeys.length}
          reset={query?.twoFactorReset === "1"}
        />
      }
      passkeys={
        <AdminPasskeys
          data={passkeys}
          isSelf={isSelf}
          userId={id}
          email={user.email ?? "이메일 없음"}
          removed={query?.passkeysRemoved === "1"}
        />
      }
      detailActions={
        <UserDetailActions
          id={user.id}
          isSelf={isSelf}
          enabled={user.enabled}
        />
      }
    />
  );
}
