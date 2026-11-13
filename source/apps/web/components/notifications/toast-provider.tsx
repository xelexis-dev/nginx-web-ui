"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Button, Icon } from "@repo/ui";
import type { NotificationSeverity } from "@/lib/notification-presentation";
import styles from "./toast-provider.module.css";

export interface ToastInput {
  /** A newer result with this key supersedes the previous result. */
  key?: string;
  persistent?: boolean;
  linkLabel?: string;
  severity: NotificationSeverity;
  title: string;
  body?: string | null;
  /** Command diagnostics are shown as a bounded preformatted pane, regardless of severity. */
  output?: string | null;
  link?: string | null;
}
interface ToastItem extends ToastInput {
  id: string;
}
interface ToastCtx {
  toast: (t: ToastInput) => void;
}
interface ToastQueueCtx {
  items: ToastItem[];
  dismiss: (id: string) => void;
}

const Ctx = createContext<ToastCtx | null>(null);
// 큐(현재 떠 있는 토스트)는 별도 컨텍스트. `toast()`만 쓰는 소비자(bell 등)가
// 토스트가 뜨고 질 때마다 리렌더되지 않도록 분리한다. <ToastViewport />만 구독.
const QueueCtx = createContext<ToastQueueCtx | null>(null);
const AUTO_DISMISS_MS = 5000;

export function useToast(): ToastCtx {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useToast must be used within ToastProvider");
  return ctx;
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const nextId = useRef(1);
  // toastId → auto-dismiss 타이머. Map으로 들고 있어야 수동 dismiss(X/클릭) 시
  // 그 토스트의 대기 중 타이머도 함께 정리할 수 있다(안 그러면 이미 사라진 id로
  // dismiss가 한 번 더 도는 no-op 타이머가 최대 5초간 남는다 — 리뷰 A6).
  const timers = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  const dismiss = useCallback((id: string) => {
    const timer = timers.current.get(id);
    if (timer) {
      clearTimeout(timer);
      timers.current.delete(id);
    }
    setItems((xs) => xs.filter((x) => x.id !== id));
  }, []);

  const toast = useCallback(
    (t: ToastInput) => {
      const id = t.key ? `key:${t.key}` : `toast:${nextId.current++}`;
      const previous = timers.current.get(id);
      if (previous) clearTimeout(previous);
      timers.current.delete(id);
      setItems((xs) => [...xs.filter((x) => x.id !== id), { ...t, id }]);
      if (!t.persistent && t.severity !== "error") {
        const timer = setTimeout(() => dismiss(id), AUTO_DISMISS_MS);
        timers.current.set(id, timer);
      }
    },
    [dismiss],
  );

  useEffect(() => {
    const pending = timers.current;
    return () => {
      for (const timer of pending.values()) clearTimeout(timer);
      pending.clear();
    };
  }, []);

  const api = useMemo(() => ({ toast }), [toast]);
  const queue = useMemo(() => ({ items, dismiss }), [items, dismiss]);

  return (
    <Ctx.Provider value={api}>
      <QueueCtx.Provider value={queue}>{children}</QueueCtx.Provider>
    </Ctx.Provider>
  );
}

/** One viewport follows the native modal top layer; its queue stays in the provider. */
export function ToastViewport() {
  const router = useRouter();
  const ctx = useContext(QueueCtx);
  if (!ctx) throw new Error("ToastViewport must be used within ToastProvider");
  const { items, dismiss } = ctx;
  const viewportRef = useRef<HTMLDivElement>(null);
  const [modal, setModal] = useState<HTMLDialogElement | null>(null);
  useEffect(() => {
    // Native modal dialogs make their outside subtree inert, regardless of z-index.
    const locate = () =>
      setModal(
        [...document.querySelectorAll<HTMLDialogElement>("dialog:modal")].at(
          -1,
        ) ?? null,
      );
    const observer = new MutationObserver(locate);
    observer.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ["open"],
    });
    locate();
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const position = () => {
      viewport.style.removeProperty("bottom");
      const focused = document.activeElement;
      // Keep dialog actions usable even when a disabled submit leaves focus on body.
      const avoid = [
        ...(modal?.querySelectorAll<HTMLElement>("[data-toast-avoid]") ?? []),
      ];
      if (
        focused instanceof HTMLElement &&
        !focused.matches("body, html, dialog") &&
        !viewport.contains(focused)
      ) {
        avoid.push(focused);
      }
      const bounds = avoid
        .filter((el) => getComputedStyle(el).visibility !== "hidden")
        .map((el) => el.getBoundingClientRect())
        .filter((b) => b.width > 0 && b.height > 0)
        .sort((a, b) => b.top - a.top);
      // Work upward so moving above one control cannot cover a higher control.
      for (const b of bounds) {
        const a = viewport.getBoundingClientRect();
        if (
          a.left < b.right &&
          a.right > b.left &&
          a.top < b.bottom &&
          a.bottom > b.top
        ) {
          viewport.style.bottom = `${window.innerHeight - b.top + 12}px`;
        }
      }
    };
    position();
    document.addEventListener("focusin", position);
    document.addEventListener("scroll", position, true);
    window.addEventListener("resize", position);
    return () => {
      document.removeEventListener("focusin", position);
      document.removeEventListener("scroll", position, true);
      window.removeEventListener("resize", position);
    };
  }, [items, modal]);

  // Each toast owns its implicit live region; a live viewport would announce it twice.
  const viewport = (
    <div className={styles.anchor} ref={viewportRef}>
      <div className={styles.viewport}>
        {items.map((t) => (
          <div
            key={t.id}
            className={
              t.link && !t.linkLabel
                ? `${styles.toast} ${styles.clickable}`
                : styles.toast
            }
            role={t.severity === "error" ? "alert" : "status"}
            onClick={
              t.link && !t.linkLabel
                ? () => {
                    router.push(t.link!);
                    dismiss(t.id);
                  }
                : undefined
            }
          >
            <span
              className={`${styles.bar} ${styles[t.severity] ?? ""}`.trim()}
              aria-hidden
            >
              <Icon
                name={
                  t.severity === "success"
                    ? "check"
                    : t.severity === "info"
                      ? "bell"
                      : "alert"
                }
                size={16}
              />
            </span>
            <div className={styles.content}>
              <p className={styles.title}>{t.title}</p>
              {t.output || t.body ? <ToastContent body={t.body} output={t.output} /> : null}
              {t.link && t.linkLabel ? (
                <Link
                  href={t.link}
                  className={styles.action}
                  onClick={() => dismiss(t.id)}
                >
                  {t.linkLabel}
                </Link>
              ) : null}
            </div>
            <Button
              variant="ghost"
              type="button"
              className={styles.close}
              aria-label="닫기"
              onClick={(e) => {
                e.stopPropagation();
                dismiss(t.id);
              }}
            >
              <Icon name="close" />
            </Button>
          </div>
        ))}
      </div>
    </div>
  );
  return modal ? createPortal(viewport, modal) : viewport;
}

function ToastContent({ body, output }: Pick<ToastInput, "body" | "output">) {
  const ref = useRef<HTMLElement | null>(null);
  const [overflowing, setOverflowing] = useState(false);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => setOverflowing(
      element.scrollHeight > element.clientHeight || element.scrollWidth > element.clientWidth,
    );
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [body, output]);
  const Tag = output ? "pre" : "p";
  return (
    <Tag
      ref={(element: HTMLElement | null) => { ref.current = element; }}
      className={output ? styles.output : styles.body}
      title={output ? undefined : body ?? undefined}
      tabIndex={overflowing ? 0 : undefined}
    >
      {output || body}
    </Tag>
  );
}
