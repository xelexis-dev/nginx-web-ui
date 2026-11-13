"use client";

import { useEffect, useId, useMemo, useRef, useState } from "react";
import styles from "./select.module.css";
import { Icon } from "./icon";

export interface SelectOption {
  value: string;
  label: string;
}

export interface SelectProps {
  options: SelectOption[];
  /** Controlled value. Omit for uncontrolled use (defaultValue + name). */
  value?: string;
  defaultValue?: string;
  onChange?: (value: string) => void;
  /** When set, a hidden input carries the value for native form submission. */
  name?: string;
  placeholder?: string;
  /** Force the search box on/off. Default: auto — shown when options > 5. */
  searchable?: boolean;
  disabled?: boolean;
  size?: "sm" | "md";
  id?: string;
  "aria-label"?: string;
  /** Passed to the trigger, so a Field can connect its hint and error (and mark it invalid). */
  "aria-describedby"?: string;
  "aria-invalid"?: boolean | "true" | "false";
  "aria-required"?: boolean | "true" | "false";
}

/**
 * Shared custom dropdown — the app's ONLY select control. A styled trigger +
 * popover list (tokens, Light/Dark) with optional type-ahead search and
 * keyboard support (↑/↓/Enter/Esc). Works controlled (value + onChange) or
 * uncontrolled (defaultValue + name → a hidden input for form submission).
 * Always use this instead of a native <select>.
 */
export function Select({
  options,
  value,
  defaultValue,
  onChange,
  name,
  placeholder = "선택하세요",
  searchable,
  disabled,
  size = "md",
  id,
  "aria-label": ariaLabel,
  "aria-describedby": ariaDescribedBy,
  "aria-invalid": ariaInvalid,
  "aria-required": ariaRequired,
}: SelectProps) {
  const isControlled = value !== undefined;
  const [uncontrolled, setUncontrolled] = useState(defaultValue ?? "");
  const current = isControlled ? value : uncontrolled;

  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);

  const rootRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listId = useId();

  const showSearch = searchable ?? options.length > 5;

  const filtered = useMemo(() => {
    if (!showSearch || !query.trim()) return options;
    const q = query.trim().toLowerCase();
    return options.filter((o) => o.label.toLowerCase().includes(q));
  }, [options, query, showSearch]);

  const selectedLabel = options.find((o) => o.value === current)?.label;

  useEffect(() => {
    if (!open) return;
    function onDocMouseDown(e: MouseEvent) {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
        setQuery("");
      }
    }
    document.addEventListener("mousedown", onDocMouseDown);
    return () => document.removeEventListener("mousedown", onDocMouseDown);
  }, [open]);

  useEffect(() => {
    if (open && showSearch) searchRef.current?.focus();
  }, [open, showSearch]);


  // Keep the active option in range as the filtered set changes.
  useEffect(() => {
    setActive((a) => Math.min(Math.max(a, 0), Math.max(filtered.length - 1, 0)));
  }, [filtered.length]);

  // Opening starts on the chosen option, set in the same update as `open` so the very first key
  // already acts on it. (It used to start on the first option, so Enter replaced the value.)
  function openList() {
    const selected = filtered.findIndex((o) => o.value === current);
    setActive(selected >= 0 ? selected : 0);
    setOpen(true);
  }

  // Closing from inside the list returns focus to the trigger: the search box unmounts, and focus
  // would otherwise fall to <body>, losing the keyboard user's place.
  function closeList({ refocus }: { refocus: boolean }) {
    setOpen(false);
    setQuery("");
    if (refocus) triggerRef.current?.focus();
  }

  function commit(v: string) {
    if (!isControlled) setUncontrolled(v);
    onChange?.(v);
    closeList({ refocus: true });
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if (disabled) return;
    if (!open) {
      if (e.key === "Enter" || e.key === " " || e.key === "ArrowDown") {
        e.preventDefault();
        openList();
      }
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      closeList({ refocus: true });
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const opt = filtered[active];
      if (opt) commit(opt.value);
    }
  }

  return (
    <div className={styles.wrap} data-size={size} ref={rootRef} onKeyDown={onKeyDown}>
      {name ? <input type="hidden" name={name} value={current} /> : null}
      <button
        type="button"
        id={id}
        className={styles.trigger}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listId}
        aria-label={ariaLabel}
        aria-describedby={ariaDescribedBy}
        aria-invalid={ariaInvalid}
        aria-required={ariaRequired}
        disabled={disabled}
        ref={triggerRef}
        // Closing clears the search like every other close path (Esc, outside click, pick), so a
        // reopened list is the full list again. The trigger keeps focus, so no refocus is needed.
        onClick={() => (open ? closeList({ refocus: false }) : openList())}
      >
        <span className={selectedLabel ? styles.value : styles.placeholder}>
          {selectedLabel ?? placeholder}
        </span>
        <span className={styles.chevron} aria-hidden>
          <Icon name="chevronDown" size={16} />
        </span>
      </button>

      {open ? (
        <div className={styles.popover}>
          {showSearch ? (
            <input
              ref={searchRef}
              className={styles.search}
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                setActive(0);
              }}
              placeholder="검색..."
              aria-label="검색"
            />
          ) : null}
          <ul className={styles.list} role="listbox" id={listId}>
            {filtered.length === 0 ? (
              <li className={styles.empty}>결과 없음</li>
            ) : (
              filtered.map((o, i) => (
                <li key={o.value} role="option" aria-selected={o.value === current}>
                  <button
                    type="button"
                    className={i === active ? `${styles.option} ${styles.optionActive}` : styles.option}
                    onMouseEnter={() => setActive(i)}
                    onClick={() => commit(o.value)}
                  >
                    <span className={styles.optionLabel}>{o.label}</span>
                    {o.value === current ? (
                      <span className={styles.check} aria-hidden>
                        <Icon name="check" size={16} />
                      </span>
                    ) : null}
                  </button>
                </li>
              ))
            )}
          </ul>
        </div>
      ) : null}
    </div>
  );
}
