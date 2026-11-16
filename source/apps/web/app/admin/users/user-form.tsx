"use client";
import { Card, FormStack, FormBar, formFieldsClassName } from "@repo/ui";
import resourceStyles from "@/components/resource-layout.module.css";
import { FormErrors } from "@/components/form-errors";
import { useActionState, useRef, useState } from "react";
import Link from "next/link";
import {
  TextInput,
  buttonClassName,
  Button,
  Select,
  Field,
  Banner,
} from "@repo/ui";
import type { BasicFormState as ActionState } from "@repo/contracts";
import { useApiAction } from "@/lib/use-api-action";

const INITIAL: ActionState = { ok: false };

/**
 * Create page form for a new user (email, password, role). On success
 * `createUser` redirects to the new user's detail page — this component
 * never sees an `ok: true` result. Only field-validation / email-uniqueness
 * errors round-trip back here via `useActionState`.
 */
export function UserForm() {
  const [state, formAction, pending] = useActionState(
    useApiAction<ActionState>({ id: "admin.users.create", args: [] }),
    INITIAL,
  );
  const [email, setEmail] = useState("");
  const [role, setRole] = useState("viewer");
  const [mismatch, setMismatch] = useState<string | null>(null);
  const passwordRef = useRef<HTMLInputElement>(null);
  const confirmRef = useRef<HTMLInputElement>(null);

  return (
    <form
      action={formAction}
      className={resourceStyles.form}
      onSubmit={(event) => {
        if (passwordRef.current?.value !== confirmRef.current?.value) {
          event.preventDefault();
          setMismatch("비밀번호가 일치하지 않습니다.");
          return;
        }
        setMismatch(null);
      }}
    >
      <FormErrors
        errors={{
          ...state.fieldErrors,
          ...(mismatch ? { confirmPassword: mismatch } : {}),
        }}
      />
      <FormStack>
        <Banner
          tone="info"
          title={<>이메일, 비밀번호, 역할을 지정해 새 계정을 생성합니다.</>}
        />
        <Card
          id="admin-user-create"
          title={<>새 사용자</>}
          errorCount={
            Object.values({
              ...state.fieldErrors,
              ...(mismatch ? { confirmPassword: mismatch } : {}),
            }).filter(Boolean).length
          }
        >
          <div className={formFieldsClassName({ columns: 2 })}>
            <Field
              label="이메일"
              htmlFor="user-email"
              error={state.fieldErrors?.email}
            >
              <TextInput
                name="email"
                value={email}
                onChange={(event) => setEmail(event.target.value)}
                type="email"
                placeholder="user@example.com"
                autoComplete="off"
                required
              />
            </Field>

            <Field
              label="비밀번호 (8자 이상, 영문·숫자 포함)"
              htmlFor="user-password"
              error={state.fieldErrors?.password}
            >
              <TextInput
                ref={passwordRef}
                name="password"
                type="password"
                autoComplete="new-password"
                minLength={8}
                required
              />
            </Field>

            <Field
              label="비밀번호 확인"
              htmlFor="user-password-confirm"
              error={
                [mismatch, state.fieldErrors?.confirmPassword]
                  .filter(Boolean)
                  .join(" ") || undefined
              }
            >
              <TextInput
                ref={confirmRef}
                name="confirmPassword"
                type="password"
                autoComplete="new-password"
                minLength={8}
                required
              />
            </Field>

            <Field
              label="역할"
              htmlFor="user-role"
              error={state.fieldErrors?.role}
            >
              <Select
                name="role"
                value={role}
                onChange={setRole}
                options={[
                  { value: "admin", label: "admin" },
                  { value: "editor", label: "editor" },
                  { value: "viewer", label: "viewer" },
                ]}
                aria-label="역할"
              />
            </Field>

            {state.error ? (
              <Banner tone="error" role="alert" title={state.error} data-wide />
            ) : null}
          </div>
        </Card>
        <FormBar>
          <Link
            href="/admin/users"
            className={buttonClassName({ variant: "secondary" })}
          >
            취소
          </Link>
          <Button type="submit" disabled={pending}>
            {pending ? "생성 중..." : "생성"}
          </Button>
        </FormBar>
      </FormStack>
    </form>
  );
}
