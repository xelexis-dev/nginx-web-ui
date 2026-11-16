"use client";

import { useActionState } from "react";
import Link from "next/link";
import {
  Banner,
  Button,
  Field,
  Card,
  FormStack,
  FormBar,
  TextInput,
  buttonClassName,
  Checkbox,
  formFieldsClassName,
} from "@repo/ui";
import { useApiAction } from "@/lib/use-api-action";
import { SixDigitCodeInput } from "@/components/auth/six-digit-code-input";
import type { AccountState } from "../form-types";
import { BackupCodesCard } from "./backup-codes-card";
import { FormErrors } from "@/components/form-errors";
import styles from "./two-factor-management.module.css";

interface RegenerateState extends AccountState {
  backupCodes?: string[];
}
const INITIAL: RegenerateState = { ok: false };

export function RegenerateBackupCodesForm({
  hasPassword,
}: {
  hasPassword: boolean;
}) {
  const [state, action, pending] = useActionState(
    useApiAction<RegenerateState>({
      id: "account.regenerateBackupCodes",
      args: [],
    }),
    INITIAL,
  );
  if (state.ok && state.backupCodes)
    return <BackupCodesCard codes={state.backupCodes} context="regenerated" />;
  const field = hasPassword ? "currentPassword" : "code";
  const fieldError = state.fieldErrors?.[field];
  return (
    <form action={action} className={styles.form} aria-busy={pending}>
      <FormErrors
        errors={state.fieldErrors}
        targets={{ [field]: "regenerate-proof" }}
      />
      <FormStack>
        <Banner
          tone="info"
          title="인증 앱 설정은 유지하고 백업 코드만 새로 발급합니다."
        />
        <Card
          id="two-factor-regenerate"
          title="본인 확인"
          errorCount={
            Object.values(state.fieldErrors ?? {}).filter(Boolean).length
          }
        >
          <div className={formFieldsClassName({ columns: 1 })}>
            <div data-wide>
              <Banner tone="warning" title="기존 백업 코드가 모두 무효화됩니다">
                새로 발급하면 사용하지 않은 기존 백업 코드도 사용할 수 없습니다.
                새 코드는 다음 화면에서 한 번만 표시됩니다.
              </Banner>
            </div>
            <Field
              htmlFor="regenerate-proof"
              label={hasPassword ? "현재 비밀번호" : "인증 앱의 6자리 코드"}
              error={fieldError}
              required
              hint={
                hasPassword
                  ? "현재 비밀번호로 본인을 확인합니다."
                  : "비밀번호가 없는 계정입니다. 인증 앱의 새 코드를 입력하세요. 백업 코드는 사용할 수 없습니다."
              }
            >
              {hasPassword ? (
                <TextInput
                  id="regenerate-proof"
                  name="currentPassword"
                  type="password"
                  autoComplete="current-password"
                  required
                  disabled={pending}
                />
              ) : (
                <SixDigitCodeInput
                  id="regenerate-proof"
                  name="code"
                  autoComplete="one-time-code"
                  required
                  disabled={pending}
                />
              )}
            </Field>
            <Checkbox
              data-wide
              required
              disabled={pending}
              label="기존 백업 코드가 모두 무효화되는 것을 확인했습니다."
            />
            {state.error && !fieldError ? (
              <div data-wide>
                <Banner tone="error" title={state.error} role="alert" />
              </div>
            ) : null}
          </div>
        </Card>
        <FormBar>
          <Link
            href="/account/2fa"
            className={buttonClassName({ variant: "secondary" })}
          >
            취소
          </Link>
          <Button type="submit" disabled={pending}>
            {pending ? "재발급 중..." : "백업 코드 재발급"}
          </Button>
        </FormBar>
      </FormStack>
    </form>
  );
}
