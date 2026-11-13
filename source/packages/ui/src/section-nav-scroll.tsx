"use client";

import { useEffect, useRef, type ReactNode } from "react";

/** Keeps the server-rendered links and only owns scrolling and keyboard focus. */
export function SectionNavScroll({
  active,
  className,
  label,
  children,
}: {
  active: string | null;
  className: string;
  label: string;
  children: ReactNode;
}) {
  const ref = useRef<HTMLElement>(null);
  useEffect(() => {
    const nav = ref.current;
    if (!nav) return;
    const reveal = () => {
      const link = nav.querySelector<HTMLElement>('[aria-current="page"]');
      if (!link) return;
      const outer = nav.getBoundingClientRect();
      const inner = link.getBoundingClientRect();
      if (inner.left < outer.left) nav.scrollLeft += inner.left - outer.left;
      else if (inner.right > outer.right)
        nav.scrollLeft += inner.right - outer.right;
    };
    reveal();
    const observer = new ResizeObserver(reveal);
    observer.observe(nav);
    return () => observer.disconnect();
  }, [active]);
  return (
    <nav
      ref={ref}
      className={className}
      data-variant="tabs"
      aria-label={label}
      onKeyDown={(event) => {
        if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey)
          return;
        const links = [
          ...event.currentTarget.querySelectorAll<HTMLAnchorElement>("a[href]"),
        ];
        const index = links.indexOf(event.target as HTMLAnchorElement);
        if (index < 0) return;
        const next =
          event.key === "Home"
            ? 0
            : event.key === "End"
              ? links.length - 1
              : event.key === "ArrowRight"
                ? (index + 1) % links.length
                : event.key === "ArrowLeft"
                  ? (index + links.length - 1) % links.length
                  : undefined;
        if (next === undefined) return;
        event.preventDefault();
        links[next]?.focus();
      }}
    >
      {children}
    </nav>
  );
}
