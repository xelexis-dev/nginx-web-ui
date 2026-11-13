import type { SessionView } from "@repo/contracts";
import { redirect } from "next/navigation";
import { readPageApi } from "./api-page";

export type { AdminUserView, AdminUserListItem, PendingUserView, GatewayView, SecurityView } from "@repo/contracts";

/** Presentation guard; every admin API independently rechecks the principal. */
export async function readAdminSession(): Promise<SessionView> {
  const session = await readPageApi<SessionView>("/api/ui/v1/session");
  if ((session.user.requires2fa && !session.user.totpEnabled) || session.user.verificationRequired) redirect("/account/2fa");
  if (session.user.role !== "admin") redirect("/login");
  return session;
}
