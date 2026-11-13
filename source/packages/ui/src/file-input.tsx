import type { ComponentProps } from "react";
import styles from "./file-input.module.css";

/**
 * Choose one local file: the browser's own `<input type="file">`, styled (DESIGN.md, Operations).
 * The native element is the control, so the keyboard, the OS file picker, `accept` and screen
 * readers behave as the browser defines them, and the name shown is the chosen file's real name.
 * The button label and the empty text come from the browser, in its language.
 *
 * It selects; it does not upload. There is no `name`: the file is never part of a form submit (the
 * API refuses file parts, apps/api/src/http_policy.rs). The consumer reads the chosen file itself
 * from `onChange` (`event.currentTarget.files?.[0]`). One file at a time (no `multiple`).
 *
 * Label, hint and error come from `Field` (or an `aria-label`); `aria-invalid` shows the error
 * border. The ref reaches the input (a prop in React 19).
 */
export type FileInputProps = Omit<ComponentProps<"input">, "type" | "value" | "defaultValue" | "name" | "multiple" | "children">;

export function FileInput({ className, ...rest }: FileInputProps) {
  // `rest` first: a caller cannot turn this into another kind of input or drop its styles.
  return <input {...rest} type="file" className={[styles.fileInput, className].filter(Boolean).join(" ")} />;
}
