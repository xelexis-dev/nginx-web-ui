import {
  OperationsShell,
  OperationsSectionNav,
} from "@/components/operations/operations-shell";
import { ADMIN_CATEGORIES } from "@/app/(app)/nav-links";
import shellStyles from "@/components/operations/operations-shell.module.css";
import type { ReactNode } from "react";
import { unstable_rethrow } from "next/navigation";
import ErrorBoundary from "@/components/error-boundary";
import { ErrorView } from "@/components/error-view";
import { readAdminSession } from "@/lib/admin-api";
import { SessionActivity } from "@/components/session-activity";

// Admin-only settings use the global rail and shared settings navigation.
// 2FA enforcement (when the policy requires it) is handled uniformly by the
// middleware (proxy.ts) via the requires2fa claim, so no gate is needed here.
export const runtime = "nodejs";

export default async function AdminLayout({
  children,
  crumbs,
}: {
  children: ReactNode;
  crumbs?: ReactNode;
}) {
  // 셸 레이아웃의 DB 조회 실패를 여기서 직접 받는다 — 이유는 (app)/layout.tsx 참고.
  // `requireRole` 은 권한이 없으면 **내부에서 redirect() 를 던진다.** 그래서
  // `unstable_rethrow` 를 먼저 불러 그 예외를 통과시킨다 — 빼먹으면 권한 없는
  // 사용자가 리다이렉트 대신 오류 화면을 보게 되고, 가드가 조용히 무력해진다.
  let session: Awaited<ReturnType<typeof readAdminSession>>;
  try {
    session = await readAdminSession();
  } catch (err) {
    unstable_rethrow(err);
    console.error("[admin-shell] 세션 확인 실패:", err);
    return <ErrorView standalone homeHref="/login" homeLabel="로그인으로" />;
  }

  return (
    <>
      <SessionActivity />
      <OperationsShell
        role={session.user.role}
        email={session.user.email ?? ""}
        target={{ kind: "local" }}
        notifications={false}
        crumbs={crumbs}
        contentHeader={
          <div className={shellStyles.shellColumn}>
            <OperationsSectionNav
              groups={ADMIN_CATEGORIES}
              variant="tabs"
              label="관리자 설정"
            />
          </div>
        }
      >
        <ErrorBoundary homeHref="/admin" homeLabel="관리자 패널로">
          {children}
        </ErrorBoundary>
      </OperationsShell>
    </>
  );
}
