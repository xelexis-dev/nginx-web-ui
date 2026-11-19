"use client";

import Link from "next/link";
import { useActionState, useState } from "react";
import {
  TextInput,
  Button,
  Field,
  Banner,
  Card,
  CodePane,
  buttonClassName,
  FormStack,
  FormBar,
  formFieldsClassName,
} from "@repo/ui";
import { useApiAction } from "@/lib/use-api-action";
import { BackupCodesCard } from "./backup-codes-card";
import type { AccountState, TwoFactorState } from "../form-types";
import { FormErrors } from "@/components/form-errors";
import styles from "./two-factor.module.css";

const ENROLL_INITIAL: TwoFactorState = { ok: false };
const DISABLE_INITIAL: AccountState = { ok: false };

/** Verify this session with an enrolled authenticator or one unused backup code. */
export function TwoFactorVerify() {
  const [state, formAction, pending] = useActionState(
    useApiAction<AccountState>({ id: "account.verifyTwoFactor", args: [] }),
    DISABLE_INITIAL,
  );
  const [code, setCode] = useState("");
  const error = state.fieldErrors?.code ?? state.error;

  return (
    <form action={formAction} className={styles.form}>
      <FormErrors
        errors={error ? { code: error } : undefined}
        targets={{ code: "verify-code" }}
      />
      <FormStack>
        <Banner
          tone="info"
          title="계속하려면 이 로그인 세션의 인증을 완료하세요."
        />
        <Card
          id="two-factor-verify"
          title="인증 코드 확인"
          errorCount={error ? 1 : 0}
        >
          <div className={formFieldsClassName({ columns: 1 })}>
            <p id="verify-code-hint" className={styles.warn}>
              인증 앱의 6자리 코드 또는 사용하지 않은 백업 코드를 입력하세요.
            </p>
            {/* The hint sits above, as the card's lead; Field links it and keeps the ids verify-code-hint / -error. */}
            <Field
              label="인증 코드 또는 백업 코드"
              htmlFor="verify-code"
              describedBy="verify-code-hint"
              error={error ? <span role="alert">{error}</span> : undefined}
            >
              <TextInput
                name="code"
                value={code}
                onChange={(event) => setCode(event.target.value)}
                autoComplete="one-time-code"
                autoCapitalize="none"
                spellCheck={false}
                required
              />
            </Field>
          </div>
        </Card>
        <FormBar>
          <Button type="submit" disabled={pending}>
            {pending ? "확인 중..." : "인증 확인"}
          </Button>
        </FormBar>
      </FormStack>
    </form>
  );
}

/**
 * Enrollment: shows the QR + secret, confirms a code to enable 2FA, then
 * reveals the single-use backup codes exactly once.
 */
export function TwoFactorEnroll({
  secret,
  qrDataUrl,
}: {
  secret: string;
  qrDataUrl: string;
}) {
  const [state, formAction, pending] = useActionState(
    useApiAction<TwoFactorState>({ id: "account.enableTwoFactor", args: [] }),
    ENROLL_INITIAL,
  );

  if (state.ok && state.backupCodes) {
    return <BackupCodesCard codes={state.backupCodes} />;
  }

  return (
    <form action={formAction} className={styles.form}>
      <FormErrors
        errors={state.error ? { code: state.error } : undefined}
        targets={{ code: "totp-code" }}
      />
      <FormStack>
        <Banner
          tone="info"
          title="인증 앱(Google Authenticator, 1Password 등)으로 QR을 스캔한 뒤, 앱에 표시된 6자리 코드를 입력해 활성화합니다."
        />
        <Card id="two-factor-qr" title="1. QR 스캔">
          <div className={styles.stepBody}>
            <div className={styles.qrBox}>
              {/* next/image 를 쓰지 않는다: data: URI 라 최적화할 원본이 없다(원격 요청도 없다). */}
              <img
                src={qrDataUrl}
                alt="2FA QR 코드"
                width={220}
                height={220}
                className={styles.qr}
              />
            </div>
            <p className={styles.secretLabel}>또는 이 키를 수동 입력:</p>
            <CodePane
              title="설정 키"
              variant="secret"
              code={secret}
              headingLevel={3}
            />
          </div>
        </Card>

        <Card
          id="two-factor-confirm"
          title="2. 코드 확인"
          errorCount={state.error ? 1 : 0}
        >
          <div className={formFieldsClassName({ columns: 1 })}>
            <input type="hidden" name="secret" value={secret} />
            <Field
              label="인증 앱의 6자리 코드"
              htmlFor="totp-code"
              error={state.error}
            >
              <TextInput
                name="code"
                inputMode="numeric"
                autoComplete="one-time-code"
                placeholder="000000"
                required
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
            {pending ? "확인 중..." : "2FA 활성화"}
          </Button>
        </FormBar>
      </FormStack>
    </form>
  );
}

/** Disable: re-authenticate with the current password, then clear 2FA. */
export function TwoFactorDisable() {
  const [state, formAction, pending] = useActionState(
    useApiAction<AccountState>({ id: "account.disableTwoFactor", args: [] }),
    DISABLE_INITIAL,
  );

  return (
    <form action={formAction} className={styles.form}>
      <FormErrors errors={state.fieldErrors} />
      <FormStack>
        <Banner
          tone="info"
          title="해제하면 기존 인증 앱과 백업 코드로 인증할 수 없고, 등록한 패스키도 모두 삭제됩니다. 이 계정의 API 토큰도 모두 폐기됩니다. 현재 비밀번호를 확인합니다."
        />
        <Card
          id="two-factor-disable"
          title="2FA 해제"
          errorCount={
            Object.values(state.fieldErrors ?? {}).filter(Boolean).length
          }
        >
          <div className={formFieldsClassName({ columns: 1 })}>
            <Field
              label="현재 비밀번호"
              htmlFor="disable-current"
              error={state.fieldErrors?.currentPassword}
            >
              <TextInput
                name="currentPassword"
                type="password"
                autoComplete="current-password"
                required
              />
            </Field>
            {state.error ? (
              <Banner tone="error" role="alert" title={state.error} />
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
          <Button
            type="submit"
            variant="danger"
            className="nwui-btn-primary"
            disabled={pending}
          >
            {pending ? "해제 중..." : "2FA 해제"}
          </Button>
        </FormBar>
      </FormStack>
    </form>
  );
}
