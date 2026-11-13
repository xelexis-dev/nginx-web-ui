import type { ComponentProps } from "react";
import styles from "./text-input.module.css";

/**
 * Text entry: the browser's own `<input>` and `<textarea>`, styled as the app's control box
 * (DESIGN.md "form-set", Operations). Everything native passes through (name, value or defaultValue,
 * required, autoComplete, inputMode, placeholder, disabled, aria-*, ref), so forms submit them as
 * they always have. Label, hint and error come from `Field`; `aria-invalid` shows the error border.
 * No hooks: server-safe.
 */

/**
 * The text-like types, and `date` (the browser's own date picker in the same box). Search has
 * `SearchField`, files `FileInput`, on/off `Switch`; checkbox, radio and hidden are not text.
 */
export type TextInputType = "text" | "email" | "password" | "number" | "url" | "tel" | "date";

export type TextInputProps = Omit<ComponentProps<"input">, "type" | "children"> & {
  /** Default "text". */
  type?: TextInputType;
};

export function TextInput({ type = "text", className, ...rest }: TextInputProps) {
  // `rest` first: a caller cannot turn this into another kind of input or drop its styles.
  return <input {...rest} type={type} className={[styles.control, className].filter(Boolean).join(" ")} />;
}

export type TextAreaProps = Omit<ComponentProps<"textarea">, "children"> & {
  /** Raw text read as data (PEM, nginx directives): caption size, same Pretendard face. */
  mono?: boolean;
};

export function TextArea({ mono = false, className, ...rest }: TextAreaProps) {
  return (
    <textarea
      {...rest}
      data-mono={mono ? "" : undefined}
      wrap={mono ? "off" : rest.wrap}
      className={[styles.control, styles.textarea, className].filter(Boolean).join(" ")}
    />
  );
}
