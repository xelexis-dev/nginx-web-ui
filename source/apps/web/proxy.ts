import { NextResponse, type NextRequest } from "next/server";
import { apiForwardHeaders, localApiUrl } from "./lib/api-location";
import { apiUnavailableResponse } from "./lib/api-unavailable";

let frameWarningLogged = false;

function withFramePolicy(response: Response): Response {
  let ancestors = "frame-ancestors 'self'";
  let external = false;
  const raw = process.env.NWUI_FRAME_ANCESTORS?.trim();
  if (raw) {
    try {
      const origins = raw.split(/\s+/).map(origin => {
        if (!/^https?:\/\/[^/?#@\\]+$/i.test(origin)) throw new Error("origin required");
        const url = new URL(origin);
        const host = url.hostname;
        // CSP host-source cannot express IPv6 literals; fail closed for the whole list.
        const validHost = host.replace(/\.$/, "").split(".").every(label => /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/i.test(label));
        if (url.username || url.password || !validHost) throw new Error("host required");
        return url.origin;
      });
      ancestors += " " + [...new Set(origins)].join(" ");
      external = true;
    } catch {
      if (!frameWarningLogged) {
        console.warn("NWUI_FRAME_ANCESTORS 값이 올바르지 않아 같은 출처 프레임만 허용합니다.");
        frameWarningLogged = true;
      }
    }
  }
  response.headers.set("Content-Security-Policy", ancestors);
  if (external) response.headers.delete("X-Frame-Options");
  else response.headers.set("X-Frame-Options", "SAMEORIGIN");
  return response;
}

/** Public documents share the frame policy; API authentication remains independent. */
export async function proxy(req: NextRequest) {
  const path = req.nextUrl.pathname;
  // Keep the previous matcher's auth bypass unchanged, including its prefix scope.
  const publicPage = path.startsWith("/login") || path.startsWith("/setup");
  return withFramePolicy(publicPage ? NextResponse.next() : await navigationResponse(req));
}

/** Navigation guard only. Every API independently authenticates and authorizes requests. */
async function navigationResponse(req: NextRequest) {
  try {
    const response = await fetch(localApiUrl("/api/ui/v1/session"), {
      headers: apiForwardHeaders(req.headers), cache: "no-store", redirect: "manual",
      signal: AbortSignal.timeout(10_000),
    });
    if (response.status === 401) return NextResponse.redirect(new URL("/login", req.nextUrl.origin));
    if (!response.ok) throw new Error("API unavailable");
    const view: unknown = await response.json();
    const user = view && typeof view === "object" && "user" in view ? view.user : null;
    if (!user || typeof user !== "object" || !("requires2fa" in user) || !("totpEnabled" in user) || !("verificationRequired" in user)
      || typeof user.requires2fa !== "boolean" || typeof user.totpEnabled !== "boolean" || typeof user.verificationRequired !== "boolean") throw new Error("Invalid session view");
    const path = req.nextUrl.pathname;
    const verificationRequired = user.verificationRequired === true;
    if (((user.requires2fa && !user.totpEnabled) || verificationRequired) && path !== "/account/2fa" && !path.startsWith("/account/2fa/")) {
      return NextResponse.redirect(new URL("/account/2fa", req.nextUrl.origin));
    }
    return NextResponse.next();
  } catch {
    // A styled document, not a bare sentence: it says what happened, retries by itself and
    // offers the retry the plain-text answer only promised.
    return apiUnavailableResponse();
  }
}

export const config = {
  matcher: ["/((?!api/|_next/static|_next/image|favicon\\.ico$|icon\\.svg$).*)"],
};
