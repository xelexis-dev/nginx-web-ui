import { redirect } from "next/navigation";
import { ApiRequestError, readAuthOptions } from "@/lib/api-client";
import { ApiUnavailableScreen } from "@/components/api-unavailable-screen";
import { BackupCodeScreen } from "@/components/auth/backup-code-screen";

// The second sign-in step with a backup code. The same guard as /login/2fa: whether a pending
// sign-in exists is asked of the API on every request (the challenge stays in its HttpOnly cookie).
export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export default async function BackupCodePage({ searchParams }: { searchParams: Promise<{ error?: string }> }) {
  let options;
  try {
    options = await readAuthOptions();
  } catch (error) {
    if (error instanceof ApiRequestError) return <ApiUnavailableScreen />;
    throw error;
  }
  if (options.firstRun) redirect("/setup");
  if (options.authenticated) redirect("/");
  if (!options.twoFactorPending) redirect("/login?error=TwoFactorExpired");

  const { error } = await searchParams;
  return <BackupCodeScreen state={error === "invalid" ? "error" : "default"} />;
}
