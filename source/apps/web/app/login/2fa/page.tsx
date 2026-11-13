import { redirect } from "next/navigation";
import { ApiRequestError, readAuthOptions } from "@/lib/api-client";
import { ApiUnavailableScreen } from "@/components/api-unavailable-screen";
import { TwoFactorScreen } from "@/components/auth/two-factor-screen";

// The second sign-in step. Whether a pending sign-in exists is asked of the API on every request
// (the challenge itself stays in its HttpOnly cookie and is never visible here).
export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export default async function TwoFactorPage({ searchParams }: { searchParams: Promise<{ error?: string }> }) {
  let options;
  try {
    options = await readAuthOptions();
  } catch (error) {
    if (error instanceof ApiRequestError) return <ApiUnavailableScreen />;
    throw error;
  }
  // The same order as /login: first run, then a valid session, win over everything.
  if (options.firstRun) redirect("/setup");
  if (options.authenticated) redirect("/");
  // No pending sign-in (never started, expired, used up, cancelled or revoked): back to the first
  // step, which says so. The query string alone never shows this form.
  if (!options.twoFactorPending) redirect("/login?error=TwoFactorExpired");

  const { error } = await searchParams;
  return <TwoFactorScreen state={error === "invalid" ? "error" : "default"} />;
}
