"use client";

import { Fragment, useEffect, useId, useRef, useState } from "react";
import { usePathname } from "next/navigation";
import { Icon } from "@repo/ui";
import Link from "next/link";
import { ThemeToggle } from "./theme-sync";
import type { Role } from "@repo/contracts";
import { ApiForm } from "@/components/api-form";
import { RoleBadge } from "@/components/role-badge";
import styles from "./profile-menu.module.css";

/** Avatar disclosure in the top bar; click outside, route change or Escape closes it. */
export function ProfileMenu({
  email,
  role,
  version,
  sha,
  fullSha,
}: {
  email: string;
  role: Role;
  version: string;
  sha: string;
  fullSha: string;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelId = useId();
  const titleId = useId();
  const pathname = usePathname();

  // The shell closes the drawer on its own on these; the flyout follows without moving focus.
  useEffect(() => setOpen(false), [pathname]);

  useEffect(() => {
    if (!open) return;
    function onClick(e: MouseEvent) {
      const root = rootRef.current;
      const target = e.target as Element;
      if (!root || root.contains(target)) return;
      setOpen(false);
    }
    // Keep Escape handled before any shell listener.
    function onKey(e: KeyboardEvent) {
      if (e.key !== "Escape") return;
      e.preventDefault();
      triggerRef.current?.focus();
      setOpen(false);
    }
    document.addEventListener("click", onClick, true);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("click", onClick, true);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [open]);

  const initial = (email.trim().charAt(0) || "?").toUpperCase();

  return (
    <div
      className={styles.wrap}
      ref={rootRef}
      onBlur={(e) => {
        // A null relatedTarget (a press on the flyout's own text, the backdrop) is left to the click.
        const next = e.relatedTarget;
        if (next && !e.currentTarget.contains(next)) setOpen(false);
      }}
    >
      <button
        ref={triggerRef}
        type="button"
        className={styles.trigger}
        aria-label={`계정 메뉴: ${email}`}
        title={email}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((o: boolean) => !o)}
      >
        <span className={styles.avatar} aria-hidden>
          {initial}
        </span>
      </button>

      <section
        id={panelId}
        className={styles.panel}
        role="dialog"
        aria-labelledby={titleId}
        hidden={!open}
      >
        <div className={styles.body}>
          <div className={styles.head}>
            <p className={styles.kicker} id={titleId}>
              로그인 계정
            </p>
            <p className={styles.headerEmail}>
              {/* Readable break points for the narrow strip beside the phone drawer. */}
              {email.split(/(?<=[@.])/).map((part, i) => (
                <Fragment key={i}>
                  {i ? <wbr /> : null}
                  {part}
                </Fragment>
              ))}
            </p>
          </div>
          <dl className={styles.facts}>
            <dt>역할</dt>
            <dd>
              <RoleBadge role={role} />
            </dd>
            <dt>버전</dt>
            <dd>{`nginx-web-ui v${version}`}</dd>
            <dt>커밋</dt>
            <dd>
              <span title={fullSha || undefined}>{sha}</span>
            </dd>
          </dl>
          <nav className={styles.links} aria-label="계정">
            <Link href="/account">
              <Icon name="account" size={16} />
              계정 설정
            </Link>
            <Link href="/account/tokens">
              <Icon name="tokens" size={16} />
              액세스 토큰
            </Link>
          </nav>
          <div className={styles.theme}>
            <span>테마</span>
            <ThemeToggle />
          </div>
          <div className={styles.action}>
            <ApiForm action={{ id: "session.signOut", args: [] }}>
              <button type="submit" className={styles.signOut}>
                <Icon name="signOut" size={16} />
                <span>모든 기기에서 로그아웃</span>
              </button>
            </ApiForm>
          </div>
        </div>
      </section>
    </div>
  );
}
