"use client";

import { useEffect, useId, useMemo, useRef, useState } from "react";
import styles from "./combobox.module.css";
import { Icon } from "./icon";

export interface ComboboxOption {
  /** 선택 시 실제로 입력되는 문자열. 빈 문자열도 유효한 값이다. */
  value: string;
  /** 목록에 보일 텍스트. 생략하면 value 를 그대로 보여준다. */
  label?: string;
  /** 오른쪽에 흐리게 붙는 한 줄 설명. */
  hint?: string;
}

export interface ComboboxProps {
  options: ComboboxOption[];
  /** 제어형 전용 — 자유 입력이라 항상 부모가 값을 소유한다. */
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  id?: string;
  /** 네이티브 폼 전송용. 값이 input 에 그대로 실린다. */
  name?: string;
  "aria-label"?: string;
  /** Passed to the input, so a Field can connect its hint and error (and mark it invalid), as Select does. */
  "aria-describedby"?: string;
  "aria-invalid"?: boolean | "true" | "false";
  "aria-required"?: boolean | "true" | "false";
}

/**
 * 자유 입력 + 제안 목록. `Select` 와 형제 관계지만 역할이 다르다 — Select 는
 * **닫힌 목록**(값이 반드시 옵션 중 하나)이고 Combobox 는 **열린 목록**이다:
 * 제안은 거들 뿐 목록에 없는 값도 그대로 입력·저장된다. nginx 헤더처럼
 * "흔한 것은 정해져 있지만 커스텀도 일상적인" 입력이 이 컴포넌트를 쓴다.
 *
 * 그래서 Select 를 확장하지 않고 따로 뒀다. Select 에 자유 입력을 열면 "값은
 * 항상 옵션 중 하나"라는 보장이 사라져 앱 전역의 호출부 가정이 흔들린다.
 *
 * 자유 입력을 지키는 규칙 세 가지:
 *  - 목록이 열려도 **미리 활성화된 옵션이 없다**(active = -1). 타이핑 도중
 *    Enter 가 입력을 다른 값으로 덮어쓰지 않는다 — 방향키로 고른 뒤에만 확정.
 *  - Esc·포커스 이탈은 목록만 닫고 **입력 텍스트를 건드리지 않는다.**
 *  - 브라우저 자동완성을 끈다. 안 끄면 네이티브 드롭다운이 제안 목록을 덮는다.
 *
 * 키보드: ↓/↑ 이동, Enter 확정, Esc 닫기, Tab 은 그대로 다음 필드로.
 */
export function Combobox({
  options,
  value,
  onChange,
  placeholder,
  disabled,
  id,
  name,
  "aria-label": ariaLabel,
  "aria-describedby": ariaDescribedBy,
  "aria-invalid": ariaInvalid,
  "aria-required": ariaRequired,
}: ComboboxProps) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);

  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listId = useId();
  const optionIdBase = useId();

  // 입력한 문자열로 거른다. 이름뿐 아니라 힌트도 본다 — "클릭재킹"으로
  // X-Frame-Options 를 찾을 수 있어야 목록이 사전 구실을 한다.
  //
  // 값이 이미 어떤 옵션과 정확히 같으면 거르지 않고 전부 보여준다. 안 그러면
  // 한 번 고른 뒤 다시 열었을 때 방금 고른 항목 하나만 남아 바꿀 수가 없다.
  const filtered = useMemo(() => {
    const q = value.trim().toLowerCase();
    if (q.length === 0) return options;
    if (options.some((o) => (o.label ?? o.value).toLowerCase() === q)) return options;
    return options.filter((o) => {
      const label = o.label ?? o.value;
      return label.toLowerCase().includes(q) || (o.hint?.toLowerCase().includes(q) ?? false);
    });
  }, [options, value]);

  useEffect(() => {
    if (!open) return;
    function onDocMouseDown(e: MouseEvent) {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    }
    document.addEventListener("mousedown", onDocMouseDown);
    return () => document.removeEventListener("mousedown", onDocMouseDown);
  }, [open]);

  // 목록이 줄어들면 활성 항목이 범위 밖으로 나간다. -1(미선택)은 유지한다.
  useEffect(() => {
    setActive((a) => (a < 0 ? -1 : Math.min(a, filtered.length - 1)));
  }, [filtered.length]);

  function commit(v: string) {
    onChange(v);
    setOpen(false);
    setActive(-1);
    inputRef.current?.focus();
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if (disabled) return;
    if (e.key === "Escape") {
      if (!open) return;
      e.preventDefault();
      setOpen(false);
      setActive(-1);
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (!open) {
        setOpen(true);
        setActive(filtered.length > 0 ? 0 : -1);
        return;
      }
      setActive((a) => Math.min(a + 1, filtered.length - 1));
      return;
    }
    if (e.key === "ArrowUp") {
      if (!open) return;
      e.preventDefault();
      // 첫 항목에서 한 번 더 올리면 미선택으로 빠져나와 입력으로 돌아간다.
      setActive((a) => Math.max(a - 1, -1));
      return;
    }
    if (e.key === "Enter") {
      // 활성 항목이 있을 때만 가로챈다. 없으면 폼의 기본 동작에 맡긴다.
      if (!open || active < 0) return;
      const opt = filtered[active];
      if (!opt) return;
      e.preventDefault();
      commit(opt.value);
    }
  }

  const activeId = open && active >= 0 && filtered[active] ? `${optionIdBase}-${active}` : undefined;

  return (
    <div className={styles.wrap} ref={rootRef}>
      <input
        ref={inputRef}
        id={id}
        name={name}
        className={styles.input}
        role="combobox"
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={activeId}
        aria-label={ariaLabel}
        aria-describedby={ariaDescribedBy}
        aria-invalid={ariaInvalid}
        aria-required={ariaRequired}
        autoComplete="off"
        autoCorrect="off"
        spellCheck={false}
        disabled={disabled}
        placeholder={placeholder}
        value={value}
        onChange={(e) => {
          onChange(e.target.value);
          setOpen(true);
          setActive(-1);
        }}
        onFocus={() => setOpen(true)}
        onBlur={(e) => {
          // 팝오버 안을 클릭한 경우는 닫지 않는다(선택이 취소돼 버린다).
          if (rootRef.current?.contains(e.relatedTarget as Node)) return;
          setOpen(false);
          setActive(-1);
        }}
        onKeyDown={onKeyDown}
      />
      <button
        type="button"
        className={styles.chevron}
        tabIndex={-1}
        aria-hidden
        disabled={disabled}
        onMouseDown={(e) => e.preventDefault()} // 입력 포커스를 뺏지 않는다
        onClick={() => {
          setOpen((o) => !o);
          inputRef.current?.focus();
        }}
      >
        <Icon name="chevronDown" size={16} />
      </button>

      {open ? (
        <div className={styles.popover}>
          <ul className={styles.list} role="listbox" id={listId}>
            {filtered.length === 0 ? (
              <li className={styles.empty}>제안 없음 — 직접 입력하면 그대로 저장됩니다</li>
            ) : (
              filtered.map((o, i) => {
                const isEmptyValue = o.value.length === 0;
                const text = o.label ?? (isEmptyValue ? "(빈 값)" : o.value);
                return (
                  <li
                    key={`${o.value}-${i}`}
                    id={`${optionIdBase}-${i}`}
                    role="option"
                    aria-selected={o.value === value}
                    className={i === active ? `${styles.option} ${styles.optionActive}` : styles.option}
                    onMouseEnter={() => setActive(i)}
                    // blur 보다 먼저 확정해야 클릭이 삼켜지지 않는다.
                    onMouseDown={(e) => {
                      e.preventDefault();
                      commit(o.value);
                    }}
                  >
                    <span className={isEmptyValue ? `${styles.optionName} ${styles.optionEmpty}` : styles.optionName}>
                      {text}
                    </span>
                    {o.hint ? <span className={styles.optionHint}>{o.hint}</span> : null}
                    {o.value === value ? (
                      <span className={styles.check} aria-hidden>
                        <Icon name="check" size={16} />
                      </span>
                    ) : null}
                  </li>
                );
              })
            )}
          </ul>
        </div>
      ) : null}
    </div>
  );
}
