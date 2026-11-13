/** Fixed local listener only: SSR never forwards browser credentials to a remote API. */
export function localApiUrl(path: string, base = process.env.API_URL ?? "http://127.0.0.1:3001"): URL {
  const target = new URL(base);
  if (target.protocol !== "http:" || !["127.0.0.1", "[::1]", "localhost"].includes(target.hostname)
    || target.username || target.password || target.pathname !== "/" || target.search || target.hash) {
    throw new Error("API_URL must identify the local API listener");
  }
  if (!path.startsWith("/api/") || path.includes("\\") || [...path].some(char => char.charCodeAt(0) <= 32)) {
    throw new Error("Invalid API path");
  }
  const url = new URL(path, target);
  if (url.origin !== target.origin || !url.pathname.startsWith("/api/")) throw new Error("Invalid API path");
  return url;
}

export function apiForwardHeaders(incoming: Headers): Headers {
  const forwarded = new Headers({ Accept: "application/json" });
  for (const name of ["cookie", "x-real-ip", "x-forwarded-host", "x-forwarded-proto"]) {
    const value = incoming.get(name);
    if (value) forwarded.set(name, value);
  }
  if (!forwarded.has("x-forwarded-host") && incoming.get("host")) forwarded.set("x-forwarded-host", incoming.get("host")!);
  return forwarded;
}
