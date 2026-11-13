import type { ActionRef, ApiErrorBody, FormResult } from "@repo/contracts";

const TRANSPORT_ERROR = "API 서버에 요청을 전달하지 못했습니다. 잠시 후 다시 시도하세요.";
const TIMEOUT_ERROR = "정해진 시간 안에 응답이 없어 요청을 중단했습니다. 잠시 후 다시 시도하세요. 이미 처리됐을 수 있으니 목록에서 결과를 먼저 확인하세요.";

/** Ordinary commands answer in well under a second; waiting minutes only hides a problem. */
export const SUBMIT_TIMEOUT_MS = 30_000;
/** Applying a configuration and certificate work legitimately run for minutes. */
export const LONG_SUBMIT_MS = 310_000;
const LONG_RUNNING = new Set([
  "apply.applyConfig", "history.rollback", "nodes.pairNode",
  "node.applyNodeConfig", "node.rollbackNodeSnapshot", "node.renewNodeCertificate", "node.replaceNodeCertificate",
]);

export function submitTimeoutMs(actionId: string): number {
  // Three 10-second webhook attempts plus retry backoff can take 34 seconds.
  if (actionId === "admin.webhooks.test") return 40_000;
  return LONG_RUNNING.has(actionId) || actionId.startsWith("certificates.") ? LONG_SUBMIT_MS : SUBMIT_TIMEOUT_MS;
}

/** Only OIDC initiation may leave this origin (see `safeProviderRedirect`). */
export function isProviderRedirectAction(actionId: string): boolean {
  return actionId === "login.oauthSignIn" || actionId === "account.startLink";
}

function failure<T>(error = TRANSPORT_ERROR, fieldErrors?: Record<string, string>): FormResult<T> {
  return { result: { ok: false, error, ...(fieldErrors ? { fieldErrors } : {}) } as T };
}

function isApiError(value: unknown): value is ApiErrorBody {
  if (!value || typeof value !== "object" || !("error" in value)) return false;
  const error = value.error;
  return !!error && typeof error === "object" && "code" in error && typeof error.code === "string"
    && "message" in error && typeof error.message === "string";
}

export async function submitApiForm<T>(ref: ActionRef, form: FormData): Promise<FormResult<T>> {
  if (!/^[a-zA-Z0-9_.-]+$/.test(ref.id) || ref.args.length > 8) return failure<T>();
  const body = new FormData();
  for (const [key, value] of form) if (key !== "_apiArgs") body.append(key, value);
  body.set("_apiArgs", JSON.stringify(ref.args));
  try {
    const response = await fetch(`/api/ui/v1/forms/${encodeURIComponent(ref.id)}`, {
      method: "POST", body, credentials: "same-origin", cache: "no-store", redirect: "error",
      headers: { Accept: "application/json" }, signal: AbortSignal.timeout(submitTimeoutMs(ref.id)),
    });
    const value: unknown = await response.json();
    if (!response.ok) {
      if (response.status === 401) return { redirectTo: "/login" };
      if (isApiError(value)) {
        if (response.status === 403 && value.error.code === "two_factor_required") return { redirectTo: "/account/2fa" };
        const fields = value.error.fieldErrors;
        const validFields = fields && typeof fields === "object" && !Array.isArray(fields)
          && Object.values(fields).every((v) => typeof v === "string") ? fields : undefined;
        return failure<T>(value.error.message, validFields);
      }
      return failure<T>();
    }
    if (!value || typeof value !== "object" || Array.isArray(value)) return failure<T>();
    const result = value as FormResult<T>;
    if (result.redirectTo !== undefined && (typeof result.redirectTo !== "string"
      || !(safeLocalRedirect(result.redirectTo) || safeProviderRedirect(ref.id, result.redirectTo)))) return failure<T>();
    if (result.refresh !== undefined && typeof result.refresh !== "boolean") return failure<T>();
    return result;
  } catch (error) {
    // A timeout is not the same as "could not send": the command may well be running.
    const name = error instanceof Error ? error.name : "";
    return failure<T>(name === "TimeoutError" || name === "AbortError" ? TIMEOUT_ERROR : TRANSPORT_ERROR);
  }
}

/** Only OIDC initiation may leave the UI; the API derives the endpoint from the configured provider. */
function safeProviderRedirect(action: string, target: string): boolean {
  if (!isProviderRedirectAction(action)) return false;
  try {
    if (/[\\\s]/.test(target) || [...target].some(char => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127)) return false;
    const url = new URL(target);
    return (url.protocol === "https:" || url.protocol === "http:") && !url.username && !url.password;
  } catch { return false; }
}

/** Navigation commands must stay on this UI origin, including after decoding. */
export function safeLocalRedirect(target: string): string | null {
  try {
    const decoded = decodeURIComponent(target);
    if (!target.startsWith("/") || decoded.startsWith("//") || decoded.includes("\\")
      || [...decoded].some((char) => char.charCodeAt(0) <= 32 || char.charCodeAt(0) === 127)) return null;
    const url = new URL(target, "https://ui.invalid");
    return url.origin === "https://ui.invalid" ? target : null;
  } catch { return null; }
}
