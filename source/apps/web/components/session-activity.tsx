"use client";

import { useEffect } from "react";
import { usePathname, useRouter } from "next/navigation";

/** Activity renews a session; an unattended open tab never extends its lifetime. */
export function SessionActivity() {
  const pathname = usePathname();
  const router = useRouter();
  useEffect(() => {
    const controller = new AbortController();
    let lastAttempt = 0;
    let pending = false;
    async function refresh() {
      if (document.visibilityState !== "visible" || pending || Date.now() - lastAttempt < 60_000) return;
      pending = true;
      lastAttempt = Date.now();
      try {
        const response = await fetch("/api/ui/v1/session/refresh", {
          method: "POST", credentials: "same-origin", cache: "no-store", redirect: "error",
          signal: controller.signal,
        });
        if (response.status === 401) router.replace("/login");
      } catch { /* The next user activity retries a transient outage. */ }
      finally { pending = false; }
    }
    void refresh();
    const options = { passive: true };
    window.addEventListener("pointerdown", refresh, options);
    window.addEventListener("keydown", refresh, options);
    window.addEventListener("focus", refresh);
    return () => {
      controller.abort();
      window.removeEventListener("pointerdown", refresh);
      window.removeEventListener("keydown", refresh);
      window.removeEventListener("focus", refresh);
    };
  }, [pathname, router]);
  return null;
}
