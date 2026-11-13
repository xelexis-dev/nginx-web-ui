"use client";

import { useState, type ClipboardEvent, type DragEvent } from "react";
import { TextInput, type TextInputProps } from "@repo/ui";
import styles from "./auth-screen.module.css";

/**
 * The authenticator code field of /login/2fa: one text input that only ever holds ASCII digits, at
 * most six. Text so a leading zero stays; nothing is typed for the user and nothing submits by
 * itself. An edit that would leave anything else (a letter, a space, a seventh digit) is refused as
 * a whole: the field keeps what it had. A paste or drop is judged on the value it would leave before
 * the browser inserts it, because `maxLength` would otherwise cut a longer paste down to six digits
 * and hide that it was wrong. Empty and partial values are allowed while typing; the native
 * `pattern`/`minLength`/`required` decide validity on submit.
 */

export const CODE_LENGTH = 6;
export const CODE_PATTERN = "[0-9]{6}";

/** Whether an edit that leaves `value` is kept: ASCII digits only, at most six (empty included). */
export function keepsCode(value: string): boolean {
  return /^[0-9]{0,6}$/.test(value);
}

/** The value inserting `text` over the selection [start, end) of `current` would leave. */
export function afterInsert(current: string, start: number, end: number, text: string): string {
  return current.slice(0, start) + text + current.slice(end);
}

export type SixDigitCodeInputProps = Omit<
  TextInputProps,
  "type" | "value" | "defaultValue" | "onChange" | "onPaste" | "onDrop" | "inputMode" | "pattern" | "minLength" | "maxLength"
>;

export function SixDigitCodeInput(props: SixDigitCodeInputProps) {
  const [value, setValue] = useState("");
  const [caret, setCaret] = useState(0);
  const [focused, setFocused] = useState(false);

  const refuseUnlessCode = (event: ClipboardEvent<HTMLInputElement> | DragEvent<HTMLInputElement>, text: string) => {
    const input = event.currentTarget;
    const start = input.selectionStart ?? input.value.length;
    const end = input.selectionEnd ?? input.value.length;
    if (!keepsCode(afterInsert(input.value, start, end, text))) event.preventDefault();
  };

  return (
    <span className={styles.code}>
    <TextInput
      {...props}
      className={[styles.codeInput, props.className].filter(Boolean).join(" ")}
      value={value}
      onChange={(event) => {
        const next = event.currentTarget.value;
        // Not kept: the state stays, and React puts the previous value back in the field.
        if (keepsCode(next)) {
          setValue(next);
          setCaret(event.currentTarget.selectionStart ?? next.length);
        }
      }}
      onPaste={(event) => refuseUnlessCode(event, event.clipboardData.getData("text"))}
      onDrop={(event) => refuseUnlessCode(event, event.dataTransfer.getData("text"))}
      onFocus={(event) => {
        setFocused(true);
        setCaret(event.currentTarget.selectionStart ?? 0);
        props.onFocus?.(event);
      }}
      onBlur={(event) => { setFocused(false); props.onBlur?.(event); }}
      onSelect={(event) => {
        setCaret(event.currentTarget.selectionStart ?? 0);
        props.onSelect?.(event);
      }}
      inputMode="numeric"
      pattern={CODE_PATTERN}
      minLength={CODE_LENGTH}
      maxLength={CODE_LENGTH}
    />
    <span className={styles.codeCells} aria-hidden="true">
      {Array.from({ length: CODE_LENGTH }, (_, index) => <span
        key={index}
        className={styles.codeCell}
        data-focus={focused && index === Math.min(caret, CODE_LENGTH - 1) || undefined}
        data-invalid={props["aria-invalid"] || undefined}
      >{value[index]}</span>)}
    </span>
    </span>
  );
}
