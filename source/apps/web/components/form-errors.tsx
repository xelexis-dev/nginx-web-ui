"use client";
import { Banner } from "@repo/ui";
import styles from "./form-errors.module.css";

export function FormErrors({
  errors,
  targets = {},
}: {
  errors?: Record<string, string>;
  targets?: Record<string, string>;
}) {
  const entries = Object.entries(errors ?? {}).filter(([, message]) => message);
  if (!entries.length) return null;
  return (
    <Banner
      tone="error"
      role="alert"
      title={
        <span className={styles.errorLinks}>
          {entries.map(([key, message]) => (
            <button
              type="button"
              className={styles.errorLink}
              key={key}
              onClick={(event) => {
                const form = event.currentTarget.closest("form");
                const field =
                  form?.querySelector<HTMLElement>(
                    `#${CSS.escape(targets[key] ?? key)}`,
                  ) ??
                  form?.querySelector<HTMLElement>(
                    `[name="${CSS.escape(key)}"]:not([type="hidden"])`,
                  ) ??
                  form?.querySelector<HTMLElement>('[aria-invalid="true"]');
                field?.scrollIntoView({ block: "center" });
                const control = field?.matches("input,textarea,button,select")
                  ? field
                  : field?.querySelector<HTMLElement>(
                      "input:not([type=hidden]),textarea,button,select",
                    );
                control?.focus({ preventScroll: true });
              }}
            >
              {message}
            </button>
          ))}
        </span>
      }
    />
  );
}
