"use client";

import { useCallback } from "react";
import { useRouter } from "next/navigation";
import type { ActionRef } from "@repo/contracts";
import { isProviderRedirectAction, submitApiForm } from "./api-transport";

export type FormAction<T> = ActionRef | ((state: T, form: FormData) => Promise<T>);

// These commands change session cookies. A document navigation discards prefetched
// RSC/auth state; push followed by refresh can reuse the old session's redirect.
const SESSION_REDIRECT_ACTIONS = new Set([
  "login.loginAction",
  "login.verifyTwoFactor",
  "login.cancelTwoFactor",
  "account.verifyTwoFactor",
  "account.finishTwoFactorSetup",
  "account.changePassword",
  "account.disableTwoFactor",
  "session.signOut",
  "login.passkeyFinish",
  "account.passkeys.finish",
  "account.passkeys.remove",
  "admin.users.passkeys.remove",
]);

/** An ActionRef crosses the RSC boundary as data; only the browser executes this callback. */
export function useApiAction<T>(action: FormAction<T>): (state: T, form: FormData) => Promise<T> {
  const router = useRouter();
  return useCallback(async (state: T, form: FormData) => {
    if (typeof action === "function") return action(state, form);
    if (action.id === "nodes.pairNode") {
      const attempt = state && typeof state === "object" && "attempt" in state ? state.attempt : 0;
      form.set("_attempt", String(typeof attempt === "number" && Number.isSafeInteger(attempt) && attempt >= 0 ? attempt : 0));
    }
    const response = await submitApiForm<T>(action, form);
    // A full navigation off this origin belongs to OIDC initiation only. The transport already
    // refuses an absolute URL from any other command; this keeps the browser from following one
    // even if that check ever moves or a response shape changes.
    const offOrigin = /^https?:\/\//.test(response.redirectTo ?? "");
    if (offOrigin && !isProviderRedirectAction(action.id)) {
      router.refresh();
      return response.result === undefined ? state : response.result;
    }
    if (response.redirectTo && (SESSION_REDIRECT_ACTIONS.has(action.id) || offOrigin)) {
      window.location.assign(response.redirectTo);
      return response.result === undefined ? state : response.result;
    }
    if (response.redirectTo) router.push(response.redirectTo);
    if (response.refresh || response.redirectTo) router.refresh();
    return response.result === undefined ? state : response.result;
  }, [action, router]);
}
