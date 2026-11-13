"use client";

import { BrandLogo } from "./brand-logo";
import { IconButton } from "./button";

import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { Icon, type IconName } from "./icon";
import styles from "./app-shell.module.css";
import { activeNavHref } from "./section-nav";
export { activeNavHref } from "./section-nav";

/** Shared global rail and responsive drawer; router behavior stays in renderLink. */

export interface AppShellLinkProps {
  href: string;
  className: string;
  children: ReactNode;
  "aria-current"?: "page";
  title?: string;
  /** Closes the drawer after a menu choice. An adapter must pass it on to its link. */
  onClick?: () => void;
}
export type RenderLink = (link: AppShellLinkProps) => ReactNode;

export interface AppShellNavItem {
  href: string;
  label: string;
  icon?: IconName;
  /** "prefix" (default): active for the href and anything under it. "exact": only the href itself. */
  match?: "prefix" | "exact";
}
export interface AppShellNavGroup {
  id: string;
  /** Visible group heading that also labels its navigation list for screen readers. */
  label: string;
  items: AppShellNavItem[];
}

export interface AppShellProps {
  nav: AppShellNavGroup[];
  /** The current pathname; the longest matching item is marked as the current page. */
  currentPath: string;
  renderLink?: RenderLink;
  /** Server-rendered breadcrumb content in the top bar. */
  topbarContext?: ReactNode;
  /** Right of the utility bar: apply, notifications and theme. */
  topbarActions?: ReactNode;
  /** Full-width strip below the top bar. */
  banner?: ReactNode;
  version?: string;
  /** Context band and section tabs before the page content. */
  contentHeader?: ReactNode;
  /** Optional extra footer content; the product account menu lives in topbarActions. */
  sidebarFooter?: ReactNode;
  navLabel?: string;
  children: ReactNode;
}

const NARROW = "(max-width: 768px)";

const defaultRenderLink: RenderLink = ({
  href,
  className,
  children,
  onClick,
  ...rest
}) => (
  <a
    href={href}
    className={className}
    aria-current={rest["aria-current"]}
    title={rest.title}
    onClick={onClick}
  >
    {children}
  </a>
);

export function AppShell({
  nav,
  currentPath,
  renderLink = defaultRenderLink,
  topbarContext,
  topbarActions,
  contentHeader,
  banner,
  version,
  sidebarFooter,
  navLabel = "주요 내비게이션",
  children,
}: AppShellProps) {
  const baseId = useId().replace(/[^A-Za-z0-9_-]/g, "");
  const sidebarId = `${baseId}-sidebar`;
  const mainId = `${baseId}-main`;
  const [open, setOpen] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [narrow, setNarrow] = useState(false);
  const toggleRef = useRef<HTMLButtonElement>(null);
  const sidebarRef = useRef<HTMLElement>(null);
  const mainRef = useRef<HTMLElement>(null);
  const shellRef = useRef<HTMLDivElement>(null);
  const topbarRef = useRef<HTMLElement>(null);
  // Where focus goes when the drawer closes: the menu button (Esc, backdrop, close), the page (a
  // menu choice), or nowhere (the window widened or the route changed on its own).
  const restoreTo = useRef<"toggle" | "main" | null>(null);
  const wasOpen = useRef(false);

  useEffect(() => {
    let value = document.documentElement.dataset.navCollapsed === "true";
    try {
      if (document.documentElement.dataset.navCollapsed === undefined)
        value = localStorage.getItem("nwui-nav-collapsed") === "true";
    } catch {
      /* Storage is optional. */
    }
    document.documentElement.dataset.navCollapsed = String(value);
    setCollapsed(value);
  }, []);

  function toggleCollapsed() {
    const value = !collapsed;
    document.documentElement.dataset.navCollapsed = String(value);
    setCollapsed(value);
    try {
      localStorage.setItem("nwui-nav-collapsed", String(value));
    } catch {
      /* Storage is optional. */
    }
  }

  function close(to: "toggle" | "main" | null) {
    restoreTo.current = to;
    setOpen(false);
  }

  useEffect(() => {
    const query = window.matchMedia(NARROW);
    const update = () => {
      setNarrow(query.matches);
      if (!query.matches) close(null);
    };
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);

  // A route change while the drawer is still open (back/forward, a link outside the menu) closes it
  // without moving focus. A menu choice has already closed it (with focus going to the page) in the
  // same batch, so `open` is false here and that focus target is left alone.
  useEffect(() => {
    if (open) close(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only on navigation
  }, [currentPath]);

  useEffect(() => {
    if (open) {
      sidebarRef.current?.querySelector<HTMLElement>("nav a[href]")?.focus();
    } else if (wasOpen.current) {
      if (restoreTo.current === "toggle") toggleRef.current?.focus();
      else if (restoreTo.current === "main") mainRef.current?.focus();
    }
    wasOpen.current = open;
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      // A popup that handled Escape closes without dismissing the drawer.
      if (event.key === "Escape" && !event.defaultPrevented) close("toggle");
      // Tab wraps inside the open drawer; everything else is inert, but without this focus would
      // leave the last link for <body> or the browser's own chrome.
      if (event.key === "Tab" && sidebarRef.current) {
        const items = [
          ...sidebarRef.current.querySelectorAll<HTMLElement>(
            "a[href], button:not([disabled])",
          ),
        ].filter((el) => el.getClientRects().length > 0);
        if (!items.length) return;
        const first = items[0]!;
        const last = items[items.length - 1]!;
        const inside = sidebarRef.current.contains(document.activeElement);
        if (event.shiftKey && (document.activeElement === first || !inside)) {
          event.preventDefault();
          last.focus();
        } else if (
          !event.shiftKey &&
          (document.activeElement === last || !inside)
        ) {
          event.preventDefault();
          first.focus();
        }
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);

  // Wrapped actions and zoom can make the utility bar taller than its minimum.
  useEffect(() => {
    const shell = shellRef.current;
    const topbar = topbarRef.current;
    if (!shell || !topbar || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() =>
      shell.style.setProperty(
        "--shell-topbar-height",
        `${topbar.offsetHeight}px`,
      ),
    );
    observer.observe(topbar);
    return () => {
      observer.disconnect();
      shell.style.removeProperty("--shell-topbar-height");
    };
  }, []);

  const drawerOpen = open && narrow;

  // Inert content does not stop the page itself from scrolling under the drawer (wheel and touch on
  // the backdrop). Lock the document while it is open and put back what was there on close, on
  // widening (drawerOpen turns false) and on unmount.
  useEffect(() => {
    if (!drawerOpen) return;
    // Both the root and <body>: the root stops the viewport in most browsers, <body> is what some
    // mobile browsers scroll. Each gets back exactly the inline value it had.
    const targets = [document.documentElement, document.body];
    const previous = targets.map((el) => el.style.overflow);
    for (const el of targets) el.style.overflow = "hidden";
    return () => {
      targets.forEach((el, i) => {
        el.style.overflow = previous[i]!;
      });
    };
  }, [drawerOpen]);

  const active = activeNavHref(currentPath, nav);

  return (
    <div
      ref={shellRef}
      className={styles.shell}
      data-drawer={drawerOpen ? "open" : "closed"}
    >
      {/* While the drawer is open everything but the drawer is inert, the skip link included. */}
      <a className={styles.skip} href={`#${mainId}`} inert={drawerOpen}>
        본문으로 건너뛰기
      </a>
      <aside
        id={sidebarId}
        ref={sidebarRef}
        className={styles.sidebar}
        aria-label={navLabel}
        role={drawerOpen ? "dialog" : undefined}
        aria-modal={drawerOpen ? true : undefined}
      >
        <div className={styles.brand}>
          <div className={styles.brandIdentity}>
            <BrandLogo />
          </div>
          <IconButton
            size="sm"
            variant="ghost"
            className={styles.drawerClose}
            label="메뉴 닫기"
            onClick={() => close("toggle")}
          >
            <Icon name="close" />
          </IconButton>
        </div>
        <nav className={styles.nav} aria-label={navLabel}>
          {nav.map((group) => {
            const labelId = `${baseId}-group-${group.id}`;
            return (
              <div key={group.id} className={styles.navGroup}>
                <p className={styles.navLabel} id={labelId}>
                  {group.label}
                </p>
                <ul className={styles.navList} aria-labelledby={labelId}>
                  {group.items.map((item) => (
                    <li key={item.href}>
                      {renderLink({
                        href: item.href,
                        className: styles.navItem!,
                        "aria-current":
                          item.href === active ? "page" : undefined,
                        title: item.label,
                        onClick: () => {
                          if (drawerOpen) close("main");
                        },
                        children: (
                          <>
                            {item.icon ? (
                              <Icon name={item.icon} size={16} />
                            ) : null}
                            <span className={styles.navText}>{item.label}</span>
                          </>
                        ),
                      })}
                    </li>
                  ))}
                </ul>
              </div>
            );
          })}
        </nav>
        <div className={styles.sidebarFoot}>
          <IconButton
            className={styles.collapseToggle}
            variant="ghost"
            size="sm"
            label={collapsed ? "메뉴 펼치기" : "메뉴 접기"}
            aria-expanded={!collapsed}
            onClick={toggleCollapsed}
          >
            <Icon name={collapsed ? "panelLeftOpen" : "panelLeftClose"} />
          </IconButton>
          {version ? (
            <span className={styles.version} title={`nginx-web-ui v${version}`}>
              v{version}
            </span>
          ) : null}
          {sidebarFooter}
        </div>
      </aside>
      <div
        className={styles.backdrop}
        aria-hidden="true"
        onClick={(event) => {
          // Honor a popup that already handled the click.
          if (!event.defaultPrevented) close("toggle");
        }}
      />
      <div className={styles.mainCol} inert={drawerOpen}>
        <header ref={topbarRef} className={styles.topbar}>
          <div className={styles.topbarInner}>
            <IconButton
              size="sm"
              variant="ghost"
              ref={toggleRef}
              type="button"
              className={styles.menuToggle}
              aria-controls={sidebarId}
              aria-expanded={drawerOpen}
              label="메뉴 열기"
              onClick={() => setOpen(true)}
            >
              <Icon name="menu" />
            </IconButton>
            <div className={styles.context}>{topbarContext}</div>
            {topbarActions ? (
              <div className={styles.topbarActions}>{topbarActions}</div>
            ) : null}
          </div>
        </header>
        {banner}
        {contentHeader ? (
          <div className={styles.contentHeader}>{contentHeader}</div>
        ) : null}
        <main id={mainId} ref={mainRef} className={styles.main} tabIndex={-1}>
          <div className={styles.content}>{children}</div>
        </main>
      </div>
    </div>
  );
}
