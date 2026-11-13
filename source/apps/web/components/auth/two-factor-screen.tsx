import Link from "next/link";
import { linkClassName, Button, Field, buttonClassName } from "@repo/ui";
import { ApiForm } from "@/components/api-form";
import { AuthFrame } from "./auth-frame";
import { SixDigitCodeInput } from "./six-digit-code-input";
import styles from "./auth-screen.module.css";

/**
 * The sign-in screen's second step (/login/2fa), for an account with 2FA whose password was just
 * accepted. No session exists yet: the API holds the pending sign-in (five minutes, a few tries) and
 * this screen carries nothing from the first step. It takes the authenticator's code: exactly six
 * ASCII digits, as text so a leading zero stays (SixDigitCodeInput: nothing else can be typed or
 * pasted, and a paste in another shape is refused whole, never rewritten), so a wrong shape never
 * spends one of the few tries. A
 * backup code has its own screen (/login/2fa/backup, BackupCodeScreen). Presentational only: the page
 * decides the state from the URL; the forms send login.verifyTwoFactor (method totp) and
 * login.cancelTwoFactor.
 */

export type TwoFactorScreenState = "default" | "error";

const ERROR_ID = "two-factor-error";

export function TwoFactorScreen({ state }: { state: TwoFactorScreenState }) {
  const wrongCode = state === "error";
  return (
    <AuthFrame
      title="2단계 인증"
      subtitle="인증 앱에 표시된 6자리 코드를 입력하세요."
    >
      <div className={styles.stack}>
        <ApiForm
          action={{ id: "login.verifyTwoFactor", args: [] }}
          className={styles.form}
        >
          <input type="hidden" name="method" value="totp" />
          <Field
            label="인증 앱의 6자리 코드"
            htmlFor="totp"
            hint="숫자 6자리만 입력합니다. 5분 안에 마치지 않으면 처음부터 다시 로그인합니다."
            describedBy={wrongCode ? ERROR_ID : undefined}
          >
            <SixDigitCodeInput
              name="totp"
              autoComplete="one-time-code"
              spellCheck={false}
              required
              autoFocus
              aria-invalid={wrongCode || undefined}
            />
          </Field>
          {wrongCode ? (
            <p className={styles.error} id={ERROR_ID} role="alert">
              코드가 올바르지 않습니다. 인증 앱의 6자리 코드를 다시 입력하세요.
            </p>
          ) : null}

          <Button type="submit" className={styles.submit}>
            인증 확인
          </Button>
        </ApiForm>

        <p className={styles.altMethod}>
          <Link href="/login/2fa/backup" className={linkClassName({ variant: "back" })}>
            백업 코드로 인증
          </Link>
        </p>

        {/* A separate form: starting again sends no code, and the API forgets the pending sign-in. */}
        <ApiForm action={{ id: "login.cancelTwoFactor", args: [] }}>
          <button
            type="submit"
            className={buttonClassName({
              variant: "secondary",
              className: styles.fullWidth,
            })}
          >
            처음부터 다시 로그인
          </button>
        </ApiForm>
      </div>
    </AuthFrame>
  );
}
