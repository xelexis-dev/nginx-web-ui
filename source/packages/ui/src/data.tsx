import type { ReactNode, HTMLAttributes, CSSProperties } from "react";
import type { Tone } from "./tone";
import styles from "./data.module.css";

/** Server-safe data display: a status label, a row of key numbers and label/value facts. */

export interface StatusBadgeProps extends Omit<
  HTMLAttributes<HTMLSpanElement>,
  "children"
> {
  size?: "sm" | "md";
  /** Solid is reserved for error classification tags (dot=false). */
  variant?: "soft" | "outline" | "solid";
  tone: Tone;
  children: ReactNode;
  /** A status shape before the text. Default true; false renders a classification tag. */
  dot?: boolean;
  /** Opt in only for a connected live log; static by default. */
  pulse?: boolean;
}

export function StatusBadge({
  tone,
  children,
  dot = true,
  pulse = false,
  size = "md",
  variant = "soft",
  className,
  ...props
}: StatusBadgeProps) {
  return (
    <span
      {...props}
      className={[styles.badge, className].filter(Boolean).join(" ")}
      data-tone={tone}
      data-size={size}
      data-variant={variant}
      data-tag={!dot ? "" : undefined}
      data-pulse={pulse ? "" : undefined}
    >
      {dot ? <span className={styles.badgeDot} aria-hidden="true" /> : null}
      {children}
    </span>
  );
}

export interface KpiRowProps {
  /** Names the group, e.g. "nginx 실시간". */
  label: string;
  items: {
    label: ReactNode;
    value: ReactNode;
    tone?: Tone;
    note?: ReactNode;
    /** Decorative context for this metric. */
    icon?: ReactNode;
  }[];
  columns?: 2 | 3 | 4;
  /** Drop the outer rules when a boxed surface already frames the readouts. */
  ruled?: boolean;
}

/** Readouts with optional decorative context; one column on narrow screens. */
export function KpiRow({ label, items, columns, ruled = true }: KpiRowProps) {
  return (
    <div className={styles.kpiClip}>
      <dl
        className={styles.kpis}
        aria-label={label}
        data-columns={columns}
        data-ruled={ruled ? undefined : "false"}
      >
        {items.map((item, index) => (
          <div key={index} className={styles.kpi} data-tone={item.tone}>
            <dt>
              <span>{item.label}</span>
              {item.icon != null ? (
                <span className={styles.kpiIcon} aria-hidden="true">
                  {item.icon}
                </span>
              ) : null}
            </dt>
            <dd>
              <span
                className={styles.kpiValue}
                style={
                  {
                    "--value-chars":
                      typeof item.value === "string" ||
                      typeof item.value === "number"
                        ? String(item.value).length
                        : 1,
                  } as CSSProperties
                }
              >
                {item.value}
              </span>
              {item.note ? <p className={styles.kpiNote}>{item.note}</p> : null}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

export interface FactsProps {
  /** The existing detail grid, or a wrapping list summary. */
  layout?: "grid" | "strip";
  items: {
    term: ReactNode;
    value: ReactNode;
    mono?: boolean;
    wide?: boolean;
    note?: ReactNode;
  }[];
  /** Pairs per row on wide screens (one on narrow screens). Default 1. */
  columns?: 1 | 2;
}

/** Label/value pairs (a <dl>), e.g. a server's settings on its detail page. */
export function Facts({ items, columns = 1, layout = "grid" }: FactsProps) {
  const notes = items.some((item) => item.note != null);
  return (
    <dl
      className={styles.facts}
      data-columns={columns}
      data-layout={layout}
      data-notes={notes ? "" : undefined}
    >
      {items.map((item, index) => (
        <div
          key={index}
          className={styles.fact}
          data-wide={item.wide ? "" : undefined}
        >
          <dt>{item.term}</dt>
          <dd className={item.mono ? styles.mono : undefined}>{item.value}</dd>
          {notes ? <dd className={styles.factNote}>{item.note}</dd> : null}
        </div>
      ))}
    </dl>
  );
}

export interface StatusDotProps {
  tone?: Tone;
  /** Legacy sizes remain accepted; status markers always render at 8px. */
  size?: 6 | 7 | 8;
  /** Opt in only for a connected live log; static by default. */
  pulse?: boolean;
  className?: string;
}
/** Decorative status marker. The caller must also provide a visible status label. */
export function StatusDot({
  tone = "neutral",
  pulse = false,
  className,
}: StatusDotProps) {
  return (
    <span
      aria-hidden="true"
      className={[styles.statusDot, className].filter(Boolean).join(" ")}
      data-tone={tone}
      data-size="8"
      data-pulse={pulse ? "" : undefined}
    />
  );
}

export interface SettingRowsProps {
  children: ReactNode;
}
export interface SettingRowProps {
  label: ReactNode;
  status?: ReactNode;
  detail?: ReactNode;
  wideDetail?: boolean;
  action?: ReactNode;
  message?: ReactNode;
}

export function SettingRows({ children }: SettingRowsProps) {
  return <ul className={styles.settingRows}>{children}</ul>;
}

export function SettingRow({
  label,
  status,
  detail,
  wideDetail = false,
  action,
  message,
}: SettingRowProps) {
  return (
    <li className={styles.settingRow}>
      <div className={styles.settingLabel}>{label}</div>
      {!wideDetail ? (
        <div className={styles.settingStatus}>{status}</div>
      ) : null}
      <div
        className={styles.settingDetail}
        data-wide={wideDetail ? "" : undefined}
      >
        {detail}
      </div>
      <div className={styles.settingAction}>{action}</div>
      {message != null ? (
        <div className={styles.settingMessage}>{message}</div>
      ) : null}
    </li>
  );
}
