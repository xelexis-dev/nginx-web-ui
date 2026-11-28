import { readOperationsShellData } from "@/lib/operations-shell-data";
import { OperationsShell } from "@/components/operations/operations-shell";
import type { ReactNode } from "react";
import { redirect, unstable_rethrow } from "next/navigation";
import { ErrorView } from "@/components/error-view";
import ErrorBoundary from "@/components/error-boundary";
import { ToastProvider } from "@/components/notifications/toast-provider";
import { ApiRequestError, readSession } from "@/lib/api-client";
import { SessionActivity } from "@/components/session-activity";

// Full app-shell: sidebar (nav, role-aware) + topbar (user/role/theme/logout)
// + main content. Pages nested here call auth() -> node runtime.
export const runtime = "nodejs";

export default async function AppLayout({
  children,
  crumbs,
}: {
  children: ReactNode;
  crumbs?: ReactNode;
}) {
  // Revalidate the session against the DB (existence + role + sessionVersion),
  // not just the signed token — a deleted or revoked user is bounced to /login
  // even while their JWT is still cryptographically valid.
  // 세션 확인은 DB 를 친다 — **DB 가 죽으면 여기서 던진다.** 그 오류는 셸 안쪽에 둔
  // 경계보다 위라 잡히지 않고, `error.tsx` 도 `global-error` 도 서버 렌더 실패에는
  // 화면을 내지 못해서 Next 기본 오류 문서가 그대로 나갔다(셋 다 실측). 그래서
  // 경계에 기대지 않고 여기서 직접 받는다.
  //
  // `unstable_rethrow` 가 먼저다. `redirect()`·`notFound()` 는 예외를 던져서 동작하므로
  // 그냥 catch 하면 **로그인 리다이렉트를 삼킨다** — 인증이 조용히 풀리는 셈이다.
  let session: Awaited<ReturnType<typeof readSession>>;
  try {
    session = await readSession();
  } catch (err) {
    unstable_rethrow(err);
    if (err instanceof ApiRequestError && err.status === 401)
      redirect("/login");
    console.error("[shell] 세션 확인 실패:", err);
    return <ErrorView standalone homeHref="/login" homeLabel="로그인으로" />;
  }

  // Admin 2FA enforcement is NOT done here. A shared layout does not re-execute
  // on client-side soft navigation, so a redirect() guard here fires only on
  // hard loads (letting soft-navs bypass it, and looping the post-login nav).
  // It lives in proxy.ts (middleware), which runs on every request with a
  // reliable pathname.

  const shellData = await readOperationsShellData();
  return (
    <ToastProvider>
      <SessionActivity />
      <OperationsShell
        role={session.user.role}
        email={session.user.email ?? ""}
        target={{ kind: "local" }}
        data={shellData}
        crumbs={crumbs}
      >
        <ErrorBoundary homeHref="/" homeLabel="개요로">
          {children}
        </ErrorBoundary>
      </OperationsShell>
    </ToastProvider>
  );
}
