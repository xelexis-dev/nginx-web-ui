// Storybook-only stand-in for `next/link` (aliased in .storybook/main.ts; never used by the app).
// Product stories render the real apps/web components, whose links point at app routes. A click never
// loads the route in the frame: it opens the matching product story, or the frame says that screen is
// not in the preview (preview-nav.tsx).
import type { AnchorHTMLAttributes, MouseEvent, ReactNode } from "react";
import { usePreviewNav } from "../preview-nav";

type Href = string | { pathname?: string | null; query?: Record<string, string> };

export interface LinkProps extends Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href"> {
  href: Href;
  prefetch?: boolean | null;
  replace?: boolean;
  scroll?: boolean;
  children?: ReactNode;
}

export default function Link({ href, prefetch: _prefetch, replace: _replace, scroll: _scroll, onClick, ...rest }: LinkProps) {
  const go = usePreviewNav();
  const target = typeof href === "string" ? href : (href.pathname ?? "#");
  return (
    <a
      {...rest}
      href={target}
      onClick={(event: MouseEvent<HTMLAnchorElement>) => {
        onClick?.(event);
        event.preventDefault();
        go(target);
      }}
    />
  );
}
