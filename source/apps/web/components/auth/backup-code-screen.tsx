import Link from "next/link";
import {
  linkClassName,
  Button,
  Field,
  TextInput,
  buttonClassName,
} from "@repo/ui";
import { ApiForm } from "@/components/api-form";
import { AuthFrame } from "./auth-frame";
import styles from "./auth-screen.module.css";

/**
 * The second step with a backup code (/login/2fa/backup), for when the authenticator is not at hand.
 * Same pending sign-in, guard, cancel and expiry as /login/2fa. One backup code (xxxxx-xxxxx, hex,
 * the dash optional, any case, surrounding spaces allowed as the API trims them), checked by the
 * browser before sending so a code in another shape (a six-digit authenticator code included) does
 * not spend one of the few tries. It is sent as typed; nothing rewrites it. Presentational only: the
 * page decides the state from the URL; the forms send login.verifyTwoFactor (method backup) and
 * login.cancelTwoFactor.
 */

export type BackupCodeScreenState = "default" | "error";

const ERROR_ID = "backup-code-error";
/** 5 + 5 hex digits with an optional dash, spaces around allowed (the browser applies it to the whole value). */
const BACKUP_CODE_PATTERN = "\\s*[0-9A-Fa-f]{5}-?[0-9A-Fa-f]{5}\\s*";

export function BackupCodeScreen({ state }: { state: BackupCodeScreenState }) {
  const wrongCode = state === "error";
  return (
    <AuthFrame
      title="백업 코드로 인증"
      subtitle="2단계 인증을 설정할 때 받은 백업 코드 하나를 입력하세요."
    >
      <div className={styles.stack}>
        <ApiForm
          action={{ id: "login.verifyTwoFactor", args: [] }}
          className={styles.form}
        >
          <input type="hidden" name="method" value="backup" />
          <Field
            label="백업 코드"
            htmlFor="backup-code"
            hint="xxxxx-xxxxx 형식이며 각 코드는 한 번만 쓸 수 있습니다. 5분 안에 마치지 않으면 처음부터 다시 로그인합니다."
            describedBy={wrongCode ? ERROR_ID : undefined}
          >
            <TextInput
              name="totp"
              inputMode="text"
              pattern={BACKUP_CODE_PATTERN}
              autoComplete="off"
              autoCapitalize="off"
              spellCheck={false}
              required
              autoFocus
              aria-invalid={wrongCode || undefined}
            />
          </Field>
          {wrongCode ? (
            <p className={styles.error} id={ERROR_ID} role="alert">
              백업 코드가 올바르지 않습니다. 사용하지 않은 백업 코드를
              입력하세요.
            </p>
          ) : null}

          <Button type="submit" className={styles.submit}>
            인증 확인
          </Button>
        </ApiForm>

        <p className={styles.altMethod}>
          <Link
            href="/login/2fa"
            className={linkClassName({ variant: "back" })}
          >
            인증 앱 코드로 인증
          </Link>
        </p>
        <p className={styles.altMethod}>
          백업 코드도 없다면 관리자에게 2FA 초기화를 요청하세요.
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
