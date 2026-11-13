import { notFound, redirect } from "next/navigation";
import { ApiRequestError, apiRead, readView } from "./api-client";

/** Page navigation only; authorization and data ownership stay in the API. */
export async function readPageView<T>(path: string, query: Record<string, string | string[] | undefined> = {}): Promise<T> {
  return pageResult(() => readView<T>(path, query));
}

export async function readPageApi<T>(path: string): Promise<T> {
  return pageResult(() => apiRead<T>(path));
}

async function pageResult<T>(read: () => Promise<T>): Promise<T> {
  try {
    return await read();
  } catch (error) {
    if (error instanceof ApiRequestError) {
      if (error.status === 401) redirect("/login");
      if (error.status === 403 && error.code === "two_factor_required") redirect("/account/2fa");
      if (error.status === 403) redirect("/login");
      if (error.status === 404) notFound();
    }
    throw error;
  }
}
