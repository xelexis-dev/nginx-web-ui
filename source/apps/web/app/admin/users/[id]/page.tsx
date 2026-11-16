import { readPageApi } from "@/lib/api-page";
import { readAdminSession, type AdminUserView } from "@/lib/admin-api";
import { UserDetailActions } from "./detail-actions";

import { AdminTwoFactor, AdminUserResultNotice } from "../admin-two-factor";
import { adminPasskeyRemovedMessage } from "@/components/passkeys/admin-passkey-removed-message";
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
  const resultMessage = [
    query?.twoFactorReset === "1"
      ? "2FA를 초기화했습니다. 인증 앱 등록, 백업 코드와 패스키를 삭제하고 사용자의 로그인을 종료했습니다."
      : null,
    query?.passkeysRemoved === "1" ? adminPasskeyRemovedMessage(isSelf) : null,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <UserDetailView
      user={user}
      isSelf={isSelf}
      result={
        resultMessage ? <AdminUserResultNotice message={resultMessage} /> : null
      }
      twoFactor={
        <AdminTwoFactor
          user={user}
          isSelf={isSelf}
          canReset={passkeys.canRemove}
          passkeyCount={passkeys.passkeys.length}
        />
      }
      passkeys={
        <AdminPasskeys
          data={passkeys}
          isSelf={isSelf}
          userId={id}
          email={user.email ?? "이메일 없음"}
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
