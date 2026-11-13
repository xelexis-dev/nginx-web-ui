import { redirect } from "next/navigation";
import { ApiRequestError, readAuthOptions } from "@/lib/api-client";
import { ApiUnavailableScreen } from "@/components/api-unavailable-screen";
import { SetupScreen } from "@/components/auth/setup-screen";

// API state must not be cached across the first-admin transition.
export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * First-run admin setup. Reachable only while the system has no users; once an
 * admin exists this redirects to /login, so the setup screen can never be used
 * to add an account after initial provisioning.
 */
export default async function SetupPage() {
  let firstRun: boolean;
  try {
    firstRun = (await readAuthOptions()).firstRun;
  } catch (error) {
    if (error instanceof ApiRequestError) return <ApiUnavailableScreen />;
    throw error;
  }
  if (!firstRun) {
    redirect("/login");
  }

  return <SetupScreen />;
}
