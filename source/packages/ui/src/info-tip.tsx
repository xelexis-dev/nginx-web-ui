"use client";

import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useReducer,
  useState,
  type ReactNode,
} from "react";
import { Icon } from "./icon";
import styles from "./info-tip.module.css";

export interface InfoTipProps {
  /** Accessible name, e.g. 페이지 설명 or 백업 코드 설명. */
  label: string;
  /** A Field supplies its control's description id. */
  id?: string;
  /** Explanatory text or paragraphs; never links or controls. */
  children: ReactNode;
}

export function infoTipReducer(
  state: { open: boolean; pinned: boolean },
  event: "open" | "leave" | "toggle" | "close",
) {
  switch (event) {
    case "open":
      return { ...state, open: true };
    case "leave":
      return state.pinned ? state : { ...state, open: false };
    case "toggle":
      return { open: !state.pinned, pinned: !state.pinned };
    case "close":
      return { open: false, pinned: false };
  }
}

export function InfoTip({ label, id, children }: InfoTipProps) {
  const generatedId = useId();
  const tipId = id ?? generatedId;
  const root = useRef<HTMLSpanElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const bubble = useRef<HTMLDivElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [{ open, pinned }, dispatch] = useReducer(infoTipReducer, {
    open: false,
    pinned: false,
  });
  const [position, setPosition] = useState({ left: 16, top: 16 });
  function cancelTimer() {
    clearTimeout(timer.current);
    timer.current = undefined;
  }
  function close() {
    cancelTimer();
    dispatch("close");
  }
  function hover(next: boolean) {
    cancelTimer();
    timer.current = setTimeout(() => {
      dispatch(next ? "open" : "leave");
    }, 400);
  }
  useEffect(() => () => clearTimeout(timer.current), []);
  useEffect(() => {
    if (!open) return;
    function outside(event: PointerEvent) {
      if (!root.current?.contains(event.target as Node)) close();
    }
    function escape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        close();
      }
    }
    document.addEventListener("pointerdown", outside);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("keydown", escape);
    };
  }, [open]);
  useLayoutEffect(() => {
    if (!open) return;
    function place() {
      if (!trigger.current || !bubble.current) return;
      const anchor = trigger.current.getBoundingClientRect();
      const { width, height } = bubble.current.getBoundingClientRect();
      const below = innerHeight - anchor.bottom;
      const above = anchor.top;
      const top = below >= above ? anchor.bottom + 8 : anchor.top - 8 - height;
      setPosition({
        left: Math.max(16, Math.min(anchor.left, innerWidth - 16 - width)),
        top: Math.max(16, Math.min(top, innerHeight - 16 - height)),
      });
    }
    place();
    window.addEventListener("scroll", place, true);
    window.addEventListener("resize", place);
    return () => {
      window.removeEventListener("scroll", place, true);
      window.removeEventListener("resize", place);
    };
  }, [open, children]);
  return (
    <span ref={root} className={styles.root}>
      <button
        ref={trigger}
        type="button"
        className={styles.trigger}
        aria-label={label}
        aria-expanded={open}
        aria-describedby={tipId}
        onPointerEnter={(event) => {
          if (event.pointerType === "mouse") hover(true);
        }}
        onPointerLeave={() => hover(false)}
        onFocus={(event) => {
          if (event.currentTarget.matches(":focus-visible")) {
            cancelTimer();
            dispatch("open");
          }
        }}
        onBlur={() => {
          if (!pinned) close();
        }}
        onClick={() => {
          cancelTimer();
          dispatch("toggle");
        }}
      >
        <Icon name="info" size={16} />
      </button>
      <div
        ref={bubble}
        id={tipId}
        role="tooltip"
        hidden={!open}
        className={styles.bubble}
        style={position}
        onPointerEnter={cancelTimer}
        onPointerLeave={() => hover(false)}
      >
        {children}
      </div>
    </span>
  );
}
