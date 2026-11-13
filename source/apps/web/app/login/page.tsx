import { redirect } from "next/navigation";
import { ApiRequestError, readAuthOptions } from "@/lib/api-client";
import { ApiUnavailableScreen } from "@/components/api-unavailable-screen";
import { LoginScreen, type LoginScreenState } from "@/components/auth/login-screen";

// Login options and current-session validity come from the API on every request.
export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export default async function LoginPage({
  searchParams,
}: {
  searchParams: Promise<{ error?: string; pending?: string }>;
}) {
  // First run (no users yet): funnel every entry point to the setup screen.
  // Middleware already redirects unauthenticated requests to /login, so this
  // one check turns "/login" into the gateway to "/setup" from any URL.
  // The API is a separate process: when it is restarting, sign-in must still say so in the
  // product''s own words instead of waiting out the timeout and falling into Next''s bare 500.
  let options;
  try {
    options = await readAuthOptions();
  } catch (error) {
    if (error instanceof ApiRequestError) return <ApiUnavailableScreen />;
    throw error;
  }
  const { firstRun, authenticated, providers, twoFactorPending } = options;
  if (firstRun) {
    redirect("/setup");
  }

  // Only redirect away when the session is genuinely valid (re-checked against
  // the DB). A stale/revoked-but-still-signed token must fall through to the
  // form here — otherwise it would ping-pong with the app layout's redirect.
  if (authenticated) {
    redirect("/");
  }

  // The password was accepted for a 2FA account and the API still holds that pending sign-in:
  // continue at the second step. Starting again is that step's own action (login.cancelTwoFactor).
  if (twoFactorPending) {
    redirect("/login/2fa");
  }

  const { error, pending } = await searchParams;
  return <LoginScreen state={loginState(error, pending)} providers={providers} passkeyAvailable={options.passkey?.available} />;
}

/**
 * Which message to show. Pending approval arrives two ways: OIDC's signIn callback returns
 * ?pending=1, and loginAction returns ?error=pending when authorize throws AccountPendingError.
 * Pending wins over any error. ?error=TwoFactorExpired is the second step ending without a session
 * (expired, too many wrong codes, cancelled or revoked): a notice, not a credential error.
 */
function loginState(error: string | undefined, pending: string | undefined): LoginScreenState {
  if (pending === "1" || error === "pending") return "pending";
  if (error === "disabled") return "disabled";
  if (error === "passkey") return "passkey-error";
  if (error === "TwoFactorExpired") return "expired";
  return error ? "error" : "default";
}
