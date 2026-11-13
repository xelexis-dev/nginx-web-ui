import { apiForwardHeaders, localApiUrl } from "./api-location";
export { localApiUrl } from "./api-location";
import { headers } from "next/headers";
import { cache } from "react";
import type { ApiErrorBody, AuthOptions, SessionView } from "@repo/contracts";

export class ApiRequestError extends Error {
  constructor(public readonly status: number, public readonly code: string, message: string) {
    super(message);
    this.name = "ApiRequestError";
  }
}


/** One read per path per server render. React owns the request scope.
 * Callers share the parsed result and must not mutate it. Cookie issuance belongs to the API. */
export function apiRead<T>(path: string): Promise<T> {
  return readOnce(path) as Promise<T>;
}

const readOnce = cache(async (path: string): Promise<unknown> => {
  const incoming = await headers();
  const forwarded = apiForwardHeaders(incoming);
  let response: Response;
  try {
    response = await fetch(localApiUrl(path), {
      method: "GET", headers: forwarded, cache: "no-store", redirect: "manual", signal: AbortSignal.timeout(15_000),
    });
  } catch (error) {
    if (error instanceof Error && (error.name === "TimeoutError" || error.name === "AbortError")) {
      throw new ApiRequestError(503, "api_timeout", "API 응답을 기다리다 시간이 초과되었습니다. 잠시 후 다시 시도하세요.");
    }
    throw new ApiRequestError(503, "api_unavailable", "API 서버에 연결하지 못했습니다.");
  }
  let value: unknown;
  try { value = await response.json(); } catch {
    throw new ApiRequestError(response.ok ? 502 : response.status, "invalid_api_response", "API 응답을 읽지 못했습니다.");
  }
  if (!response.ok) {
    const error = (value as Partial<ApiErrorBody> | null)?.error;
    throw new ApiRequestError(response.status, typeof error?.code === "string" ? error.code : "api_error",
      typeof error?.message === "string" ? error.message : "API 요청에 실패했습니다.");
  }
  return value;
});

export function readSession(): Promise<SessionView> {
  return apiRead<SessionView>("/api/ui/v1/session");
}

export function readAuthOptions(): Promise<AuthOptions> {
  return apiRead<AuthOptions>("/api/ui/v1/auth/options");
}

export function readView<T>(path: string, query: Record<string, string | string[] | undefined> = {}): Promise<T> {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (key === "path" || value === undefined) continue;
    for (const item of Array.isArray(value) ? value : [value]) search.append(key, item);
  }
  search.set("path", path);
  return apiRead<T>(`/api/ui/v1/views?${search}`);
}
