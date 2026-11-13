"use client";

import { useActionState, useState } from "react";
import { TextInput, Button, Field } from "@repo/ui";
import type { BasicFormState } from "@repo/contracts";
import { useApiAction } from "@/lib/use-api-action";
import styles from "@/components/auth/auth-screen.module.css";

const INITIAL: BasicFormState = { ok: false };
const ACTION = { id: "setup.createFirstAdmin", args: [] };

/**
 * First-run admin creation form. Reuses the login card styling for a
 * consistent auth-screen look; field/form errors surface via useActionState.
 * On success the action redirects to /login.
 */
export function SetupForm() {
  const [state, formAction, pending] = useActionState(useApiAction<BasicFormState>(ACTION), INITIAL);

  const [email, setEmail] = useState("");
  return (
    <form action={formAction} className={styles.form}>
      <Field label="관리자 이메일" htmlFor="email" error={state.fieldErrors?.email}>
        <TextInput name="email" value={email} onChange={event => setEmail(event.target.value)} type="email" autoComplete="username" required />
      </Field>
      <Field label="비밀번호" htmlFor="password" error={state.fieldErrors?.password}>
        <TextInput name="password" type="password" autoComplete="new-password" required />
      </Field>
      <Field label="비밀번호 확인" htmlFor="confirmPassword" error={state.fieldErrors?.confirmPassword}>
        <TextInput name="confirmPassword" type="password" autoComplete="new-password" required />
      </Field>

      {state.error ? (
        <p className={styles.error} role="alert">
          {state.error}
        </p>
      ) : null}

      <Button type="submit" className={styles.submit} disabled={pending}>
        {pending ? "생성 중..." : "관리자 계정 만들기"}
      </Button>
    </form>
  );
}
