"use client";

import { useActionState, type ComponentProps } from "react";
import { Banner } from "@repo/ui";
import type { ActionRef } from "@repo/contracts";
import { useApiAction } from "@/lib/use-api-action";

interface FormState {
  ok: boolean;
  error?: string;
}
const INITIAL: FormState = { ok: false };

/** Uses the existing form layout while submitting directly to the browser-session API. */
export function ApiForm({
  action,
  children,
  onSubmit,
  ...props
}: Omit<ComponentProps<"form">, "action"> & { action: ActionRef }) {
  const [state, submit, pending] = useActionState(
    useApiAction<FormState>(action),
    INITIAL,
  );
  return (
    <form
      {...props}
      action={submit}
      aria-busy={pending}
      onSubmit={(event) => {
        if (pending) event.preventDefault();
        else onSubmit?.(event);
      }}
    >
      {children}
      {state.error ? (
        <Banner
          tone="error"
          role="alert"
          title={state.error}
          style={{ marginBlock: "var(--sp-2)" }}
        />
      ) : null}
    </form>
  );
}
