"use client";

import { useEffect, useRef, useState } from "react";
import { EmptyState, Icon, StatusDot, IconButton } from "@repo/ui";
import type { NotificationSeverity } from "@/lib/notification-presentation";
import styles from "./notification-bell.module.css";

export interface NotificationItem {
  id: string;
  type: string;
  severity: NotificationSeverity;
  title: string;
  body: string | null;
  link: string | null;
  createdAt: string;
}

/** The real bell/menu view; data fetching belongs to NotificationBell. */
export function NotificationBellView({
  items,
  unread,
  onOpen,
  onNavigate,
}: {
  items: NotificationItem[];
  unread: number;
  onOpen: () => void;
  onNavigate: (href: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    function onDown(e: MouseEvent) {
      if (rootRef.current && !rootRef.current.contains(e.target as Node))
        setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        setOpen(false);
        triggerRef.current?.focus();
      }
    }
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  function toggle() {
    const next = !open;
    setOpen(next);
    if (next) onOpen();
  }

  return (
    <div className={styles.wrap} ref={rootRef}>
      <IconButton
        ref={triggerRef}
        variant="ghost"
        size="sm"
        type="button"
        className={styles.trigger}
        aria-haspopup="menu"
        aria-expanded={open}
        label="알림"
        onClick={toggle}
      >
        <Icon name="bell" size={16} />
        {unread > 0 ? (
          <span className={styles.badge}>{unread > 99 ? "99+" : unread}</span>
        ) : null}
      </IconButton>
      {open ? (
        <div className={styles.menu} role="menu">
          {items.length === 0 ? (
            <EmptyState description="알림이 없습니다." />
          ) : (
            items.map((n) => {
              const inner = (
                <>
                  <StatusDot tone={n.severity} className={styles.dotPosition} />
                  <div className={styles.itemContent}>
                    <p className={styles.itemTitle}>{n.title}</p>
                    {n.body ? (
                      <p className={styles.itemBody}>{n.body}</p>
                    ) : null}
                    <p className={styles.itemAt}>
                      {n.createdAt.replace("T", " ").slice(0, 19)}
                    </p>
                  </div>
                </>
              );
              return n.link ? (
                <a
                  key={n.id}
                  href={n.link}
                  className={styles.item}
                  role="menuitem"
                  onClick={(e) => {
                    e.preventDefault();
                    setOpen(false);
                    onNavigate(n.link!);
                  }}
                >
                  {inner}
                </a>
              ) : (
                <div
                  key={n.id}
                  className={styles.item}
                  role="menuitem"
                  tabIndex={-1}
                >
                  {inner}
                </div>
              );
            })
          )}
        </div>
      ) : null}
    </div>
  );
}
