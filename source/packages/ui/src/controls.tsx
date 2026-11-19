"use client";

import { useId, type ButtonHTMLAttributes, type ReactNode } from "react";
import { Icon } from "./icon";
import styles from "./controls.module.css";

/** Filter by state with counts: toggle buttons (aria-pressed), one pressed at a time. */
export interface FilterChipsProps {
  label: string;
  options: { value: string; label: string; count?: number }[];
  value: string;
  onChange: (value: string) => void;
  /** Disables every chip, as SegmentedControl does (e.g. while a list is loading). */
  disabled?: boolean;
}

export function FilterChips({ label, options, value, onChange, disabled = false }: FilterChipsProps) {
  return (
    <div className={styles.chips} role="group" aria-label={label}>
      {options.map((option) => (
        <button key={option.value} type="button" className={styles.chip} aria-pressed={option.value === value} disabled={disabled} onClick={() => onChange(option.value)}>
          {option.label}
          {option.count !== undefined ? <span className={styles.chipCount}>{option.count}</span> : null}
        </button>
      ))}
    </div>
  );
}

/** A search box with a leading icon. The label is for screen readers (the icon and placeholder show it). */
export interface SearchFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  size?: "sm" | "md";
  id?: string;
}

export function SearchField({ label, value, onChange, placeholder, id, size = "md" }: SearchFieldProps) {
  const fallback = useId();
  const inputId = id ?? `search-${fallback.replace(/[^A-Za-z0-9_-]/g, "")}`;
  return (
    <div className={styles.search} data-size={size}>
      <label htmlFor={inputId} className={styles.srOnly}>
        {label}
      </label>
      <Icon name="search" />
      <input id={inputId} className={styles.searchInput} type="search" value={value} placeholder={placeholder} onChange={(event) => onChange(event.target.value)} />
    </div>
  );
}

/**
 * On/off as a real `<button role="switch" aria-checked>`. Controlled with onCheckedChange, or a
 * submit switch in a form (type="submit" with name/value/form) that the server flips.
 */
export interface SwitchProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "onChange" | "role" | "children"> {
  checked: boolean;
  /** Accessible name, e.g. "app.example.test 활성". */
  label: string;
  onCheckedChange?: (next: boolean) => void;
  /** Also show "활성" / "비활성" next to the track. */
  showState?: boolean;
  /** A fixed visible word beside the track, e.g. "사용" or "강제". */
  visibleLabel?: string;
  size?: "sm" | "md";
}

export function Switch({ checked, label, onCheckedChange, showState = false, visibleLabel, size = "md", type = "button", onClick, className, ...rest }: SwitchProps) {
  return (
    <button
      {...rest}
      type={type}
      data-size={size}
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={[styles.switch, className].filter(Boolean).join(" ")}
      onClick={(event) => {
        onClick?.(event);
        if (!event.defaultPrevented) onCheckedChange?.(!checked);
      }}
    >
      <span className={styles.track} aria-hidden="true">
        <span className={styles.thumb} />
      </span>
      {visibleLabel || showState ? <span className={styles.state}>{visibleLabel ?? (checked ? "활성" : "비활성")}</span> : null}
    </button>
  );
}

/**
 * One value out of a few, side by side: a named group of buttons, the chosen one pressed
 * (`aria-pressed`). For a setting inside a form (「해제 / 강제」, 「시스템 / 라이트 / 다크」); a filter with
 * counts is `FilterChips`, a plain on/off with no wording of its own is `Switch`. The labels are the
 * caller's. Controlled: the parent holds `value` (submit it with a hidden input if the form needs it).
 */
export interface SegmentedControlProps<T extends string | boolean> {
  /** Names the group for screen readers. */
  label: string;
  options: { value: T; label: ReactNode }[];
  value: T;
  onChange: (value: T) => void;
  disabled?: boolean;
  /** Compact toolbars use sm (28px); regular forms use md (32px). */
  size?: "sm" | "md";
}

export function SegmentedControl<T extends string | boolean>({ label, options, value, onChange, disabled = false, size = "md" }: SegmentedControlProps<T>) {
  return (
    <div className={styles.segmented} data-size={size} role="group" aria-label={label}>
      {options.map((option) => (
        <button
          key={String(option.value)}
          type="button"
          className={styles.segment}
          aria-pressed={option.value === value}
          disabled={disabled}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
