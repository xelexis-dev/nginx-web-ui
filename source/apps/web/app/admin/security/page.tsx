import { FormStack, Banner } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { readPageApi } from "@/lib/api-page";
import type { SecurityView } from "@/lib/admin-api";
import { SecurityForm } from "./security-form";
import { ConnectedGateways } from "./connected-gateways";

// Admin-only 2FA enforcement policy. requireRole gates the route server-side
// (defense in depth alongside the sidebar hiding it and the action's own guard).
export const runtime = "nodejs";

export default async function SecurityPage() {
  const { policy, gateways } = await readPageApi<SecurityView>(
    "/api/ui/v1/admin/security",
  );

  return (
    <section className={layout.page}>
      <PageHeader
        title={<>보안 — 2FA 강제</>}
        description={
          <>
            기본값은 모두 <strong>해제</strong>이며, 강제로 바꾸면 해당 역할
            사용자는 다음 로그인 시 2FA 등록(계정 → 2FA)을 마쳐야 합니다.
          </>
        }
      />
      <FormStack>
        <Banner
          tone="info"
          title={<>역할별로 2단계 인증(2FA)을 강제할지 설정합니다.</>}
        />
        <SecurityForm
          requireAdmin={policy.require2faAdmin}
          requireUser={policy.require2faUser}
          requireOauth={policy.require2faOauth}
        />
      </FormStack>
      <ConnectedGateways rows={gateways} />
    </section>
  );
}
