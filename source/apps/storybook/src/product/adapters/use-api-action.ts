// Storybook-only stand-in for apps/web `lib/use-api-action` (aliased in .storybook/main.ts; never
// used by the app). A product story is a preview with no API behind it: a function action (a node
// callback, or a story's own sample handler) still runs, and a command reference gets a clear answer
// instead of a network error. Nothing is saved and no request is sent.
import type { ActionRef } from "@repo/contracts";

export type FormAction<T> = ActionRef | ((state: T, form: FormData) => Promise<T>);

declare global {
  interface Window {
    /** Isolated browser fixtures may supply failed/retried/successful preview responses. */
    __NWUI_PREVIEW_ACTION__?: (action: ActionRef | null, state: unknown, form: FormData) => Promise<unknown>;
  }
}


// Sign-in commands say what did not happen; everything else keeps the general answer.
const PREVIEW_ANSWER: Record<string, string> = {
  "session.signOut": "미리보기에서는 로그아웃하지 않습니다. 요청을 보내지 않았습니다.",
  "account.regenerateBackupCodes": "미리보기에서는 백업 코드를 새로 발급하지 않습니다. 요청을 보내지 않았습니다.",
  "login.loginAction": "미리보기에서는 로그인하지 않습니다. 요청을 보내지 않았습니다.",
  "login.oauthSignIn": "미리보기에서는 로그인 공급자로 이동하지 않습니다. 요청을 보내지 않았습니다.",
  "login.verifyTwoFactor": "미리보기에서는 코드를 확인하지 않습니다. 요청을 보내지 않았습니다.",
  "login.cancelTwoFactor": "미리보기에서는 로그인 화면으로 돌아가지 않습니다. 요청을 보내지 않았습니다.",
};

export function useApiAction<T>(action: FormAction<T>): (state: T, form: FormData) => Promise<T> {
  return async (state: T, form: FormData) => {
    if (typeof window !== "undefined" && window.__NWUI_PREVIEW_ACTION__) {
      return await window.__NWUI_PREVIEW_ACTION__(typeof action === "function" ? null : action, state, form) as T;
    }
    if (typeof action === "function") return action(state, form);
    if (action.id === "node.rollbackNodeSnapshot") {
      return { ok: false, kind: "conflict", nodeVersion: null,
        message: "미리보기에서는 롤백하지 않습니다. 요청을 보내지 않았습니다." } as T;
    }
    return { ...(state as object), ok: false, error: PREVIEW_ANSWER[action.id] ?? "Storybook 미리보기에서는 저장하지 않습니다." } as T;
  };
}
