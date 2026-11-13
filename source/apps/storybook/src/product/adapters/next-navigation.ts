import { createContext, useContext } from "react";
export const StoryPathContext = createContext("/");
// Storybook-only stand-in for `next/navigation` (aliased in .storybook/main.ts; never used by the app).
// Product stories have no router: navigation and refresh are no-ops.
const router = {
  push: () => {},
  replace: () => {},
  refresh: () => {},
  back: () => {},
  forward: () => {},
  prefetch: () => {},
};

export function useRouter() {
  return router;
}

export function usePathname(): string {
  return useContext(StoryPathContext);
}

export function useSearchParams(): URLSearchParams {
  return new URLSearchParams();
}
