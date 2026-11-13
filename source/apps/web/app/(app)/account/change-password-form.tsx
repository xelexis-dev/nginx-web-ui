"use client";

import { useActionState, useId } from "react";
import Link from "next/link";
import {
  Card,
  FormStack,
  FormBar,
  buttonClassName,
  Banner,
  Field,
  TextInput,
  Button,
  formFieldsClassName,
} from "@repo/ui";
import { useApiAction } from "@/lib/use-api-action";
import type { AccountState } from "./form-types";

import styles from "./account-pages.module.css";
import { FormErrors } from "@/components/form-errors";

const initial: AccountState = { ok: false };

export function ChangePasswordForm() {
  const uid = useId();
  const [state, formAction, pending] = useActionState(
    useApiAction<AccountState>({ id: "account.changePassword", args: [] }),
    initial,
  );
  return (
    <form action={formAction} className={styles.form}>
      <FormStack>
        <FormErrors errors={state.fieldErrors} />
        <Card
          id="account-password"
          title="비밀번호 변경"
          errorCount={
            Object.values(state.fieldErrors ?? {}).filter(Boolean).length
          }
        >
          <div className={formFieldsClassName({ columns: 2 })}>
            {state.ok && (
              <div data-wide>
                <Banner tone="success" title="비밀번호가 변경되었습니다." />
              </div>
            )}
            {state.error && (
              <div data-wide>
                <Banner tone="error" role="alert" title={state.error} />
              </div>
            )}
            <Field
              label="현재 비밀번호"
              htmlFor={`${uid}-current`}
              error={state.fieldErrors?.currentPassword}
            >
              <TextInput
                name="currentPassword"
                type="password"
                autoComplete="current-password"
              />
            </Field>
            <Field
              label="새 비밀번호 (8자 이상, 영문·숫자 포함)"
              htmlFor={`${uid}-new`}
              error={state.fieldErrors?.newPassword}
            >
              <TextInput
                name="newPassword"
                type="password"
                autoComplete="new-password"
              />
            </Field>
            <Field
              label="새 비밀번호 확인"
              htmlFor={`${uid}-confirm`}
              error={state.fieldErrors?.confirmPassword}
            >
              <TextInput
                name="confirmPassword"
                type="password"
                autoComplete="new-password"
              />
            </Field>
          </div>
        </Card>
        <FormBar>
          <Link
            href="/account"
            className={buttonClassName({ variant: "secondary" })}
          >
            취소
          </Link>
          <Button type="submit" disabled={pending}>
            {pending ? "변경 중..." : "비밀번호 변경"}
          </Button>
        </FormBar>
      </FormStack>
    </form>
  );
}
