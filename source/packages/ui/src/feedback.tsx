"use client";

import {
  useEffect,
  useRef,
  useState,
  type ReactNode,
  type CSSProperties,
} from "react";
import { Button, IconButton } from "./button";
import { Icon, type IconName } from "./icon";
import type { Tone } from "./tone";
import { InfoTip } from "./info-tip";
import styles from "./feedback.module.css";

/** A message in the page (DESIGN.md "alert"): tinted by tone, with a tone icon and a full border. */
export interface BannerProps {
  style?: CSSProperties;
  id?: string;
  tone: Tone;
  title: ReactNode;
  children?: ReactNode;
  action?: ReactNode;
  /** Raw tool output (e.g. nginx -t): newlines kept, scrolls when long. */
  output?: string;
  /** Error defaults to "alert"; other tones to polite "status". */
  role?: "status" | "alert";
  onDismiss?: () => void;
  dismissLabel?: string;
  /** Decorative tone icon by default; false hides it. */
  icon?: IconName | false;
}

export function Banner({
  style,
  id,
  tone,
  title,
  children,
  action,
  output,
  role = tone === "error" ? "alert" : "status",
  onDismiss,
  dismissLabel = "알림 닫기",
  icon = tone === "success"
    ? "circleCheck"
    : tone === "error"
      ? "circleAlert"
      : tone === "info" || tone === "neutral"
        ? "info"
        : "alert",
}: BannerProps) {
  return (
    <div
      id={id}
      style={style}
      className={styles.banner}
      data-tone={tone}
      data-inline={!children && !output ? "" : undefined}
      role={role}
    >
      {icon !== false ? (
        <span className={styles.bannerIcon}>
          <Icon name={icon} size={16} />
        </span>
      ) : null}
      <div className={styles.bannerBody}>
        {tone === "info" && !children ? (
          <div className={styles.bannerText}>{title}</div>
        ) : (
          <p className={styles.bannerTitle}>{title}</p>
        )}
        {children ? <div className={styles.bannerText}>{children}</div> : null}
        {output ? (
          <pre className={styles.bannerOutput} tabIndex={0}>
            {output}
          </pre>
        ) : null}
      </div>
      {action ? <div className={styles.bannerAction}>{action}</div> : null}
      {onDismiss ? (
        <IconButton
          label={dismissLabel}
          variant="ghost"
          className={styles.dismiss}
          onClick={onDismiss}
        >
          <Icon name="close" />
        </IconButton>
      ) : null}
    </div>
  );
}

/** Generated configuration or other raw text, with an optional copy button (DESIGN.md "config"). */
export interface CodePaneProps {
  info?: ReactNode;
  title: ReactNode;
  description?: ReactNode;
  code?: string;
  /** Replaces the code, e.g. when the preview could not be rendered. */
  error?: ReactNode;
  /** Copy button label; false for none. Default "복사". */
  copyLabel?: string | false;
  /** Beside other content at ≥1024px: stays in view while the page scrolls. */
  sticky?: boolean;
  variant?: "code" | "secret";
  headingLevel?: 2 | 3;
}

export function CodePane({
  info,
  title,
  description,
  code,
  error,
  copyLabel = "복사",
  sticky = false,
  variant = "code",
  headingLevel = 2,
}: CodePaneProps) {
  const codeRef = useRef<HTMLPreElement | null>(null);
  const [overflowing, setOverflowing] = useState(false);
  useEffect(() => {
    const element = codeRef.current;
    if (!element) return;
    const measure = () =>
      setOverflowing(
        element.scrollWidth > element.clientWidth ||
          element.scrollHeight > element.clientHeight,
      );
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [code, error, variant]);
  const Heading = headingLevel === 3 ? "h3" : "h2";
  const [copied, setCopied] = useState<"idle" | "done" | "failed">("idle");
  useEffect(() => {
    if (copied === "idle" || (variant === "secret" && copied === "failed")) return;
    const t = window.setTimeout(() => setCopied("idle"), 2000);
    return () => window.clearTimeout(t);
  }, [copied, variant]);
  const lines = code?.split("\n");
  const canCopy = copyLabel !== false && code !== undefined && !error;
  return (
    <section
      className={styles.pane}
      data-sticky={sticky ? "" : undefined}
      data-variant={variant}
    >
      <div className={styles.paneHeader}>
        <div className={styles.paneHeading}>
          <div className={styles.paneTitleRow}>
            <Heading className={styles.paneTitle}>{title}</Heading>
            {info ? (
              <InfoTip
                label={typeof title === "string" ? `${title} 설명` : "설명"}
              >
                {info}
              </InfoTip>
            ) : null}
          </div>
          {description ? (
            <div className={styles.paneDescription}>{description}</div>
          ) : null}
        </div>
        {canCopy ? (
          <Button
            variant="secondary"
            size="sm"
            onClick={() => {
              if (variant === "secret") setCopied("idle");
              const done = navigator.clipboard?.writeText(code!);
              if (!done) setCopied("failed");
              else
                done.then(
                  () => setCopied("done"),
                  () => setCopied("failed"),
                );
            }}
          >
            {copied === "done" ? <Icon name="check" /> : <Icon name="copy" />}
            {copied === "done"
              ? "복사됨"
              : copied === "failed"
                ? "복사할 수 없음"
                : copyLabel}
          </Button>
        ) : null}
      </div>
      {error ? (
        <p className={styles.paneError} role="alert">
          {error}
        </p>
      ) : (
        <pre
          ref={codeRef}
          className={styles.code}
          tabIndex={overflowing ? 0 : undefined}
        >
          {lines?.map((line, index) => (
            <span className={styles.codeLine} key={index}>
              {variant !== "secret" ? (
                <span className={styles.lineNumber} aria-hidden="true">
                  {index + 1}
                </span>
              ) : null}
              <span>
                {line}
                {index < lines.length - 1 ? "\n" : null}
              </span>
            </span>
          ))}
        </pre>
      )}
      {variant === "secret" && copied === "failed" ? (
        <p className={styles.paneError} role="alert">
          복사할 수 없습니다 — 위 값을 직접 선택해 복사하세요.
        </p>
      ) : null}
    </section>
  );
}
