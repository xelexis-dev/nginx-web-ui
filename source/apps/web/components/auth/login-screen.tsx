"use client";

import { useState } from "react";
import {
  Banner,
  Button,
  Field,
  Icon,
  TextInput,
  buttonClassName,
} from "@repo/ui";
import { usePasskeyCeremony } from "@/lib/passkey-client";
import { ApiForm } from "@/components/api-form";
import { AuthFrame } from "./auth-frame";
import styles from "./auth-screen.module.css";

/**
 * The sign-in screen's first step: email and password. An account with 2FA then
 * goes to its own second step (/login/2fa, TwoFactorScreen); nothing here asks for the code.
 * Presentational only: the page decides the state from the URL and passes the providers, and each
 * form sends its real command (login.loginAction, login.oauthSignIn) through ApiForm.
 */

/** Which message the screen shows. Pending approval wins over any error (the page decides). */
export type LoginScreenState =
  "default" | "pending" | "error" | "disabled" | "expired" | "passkey-error";
export interface LoginScreenProvider {
  id: string;
  name: string;
}

const ERROR_TEXT: Record<"error" | "disabled", string> = {
  error: "이메일 또는 비밀번호가 올바르지 않습니다.",
  disabled: "비활성화된 계정입니다. 관리자에게 문의하세요.",
};
const ERROR_ID = "login-error";

export function LoginScreen({
  state,
  providers,
  passkeyAvailable,
}: {
  state: LoginScreenState;
  providers: LoginScreenProvider[];
  passkeyAvailable?: boolean;
}) {
  const passkey = usePasskeyCeremony(
    "sign-in",
    state === "passkey-error" ? "failed" : "idle",
  );
  const showPasskey =
    passkeyAvailable && (passkey.supported || state === "passkey-error");
  const [email, setEmail] = useState("");
  const errorText =
    state === "error" || state === "disabled" ? ERROR_TEXT[state] : null;
  // Only wrong credentials are about the fields' values. A disabled account is about the account:
  // its message is announced (role=alert) but the inputs are not marked invalid.
  const credentialError = state === "error";

  return (
    <AuthFrame title="로그인" subtitle="이메일과 비밀번호로 로그인하세요.">
      <div className={styles.stack}>
        {state === "pending" ? (
          <Banner tone="info" title="계정이 만들어졌습니다.">
            <strong>관리자가 승인한 뒤에</strong> 이용할 수 있습니다.
          </Banner>
        ) : null}
        {/* The second step ended without signing in: five minutes passed, the code was wrong too
            often, it was cancelled, a limit was reached, or the account changed meanwhile (signed out
            elsewhere, password or 2FA changed, disabled). Not a password error: fields not marked. */}
        {state === "expired" ? (
          <Banner tone="warning" title="로그인을 처음부터 다시 하세요.">
            2단계 인증을 마치지 못했습니다. 5분이 지났거나, 코드를 여러 번
            틀렸거나, 인증을 취소했거나, 그사이 계정 설정이 바뀌는 등의 이유로
            인증이 끝났습니다.
          </Banner>
        ) : null}

        <ApiForm
          action={{ id: "login.loginAction", args: [] }}
          className={styles.form}
        >
          <Field
            label="이메일"
            htmlFor="email"
            required
            describedBy={credentialError ? ERROR_ID : undefined}
          >
            {(control) => (
              <TextInput
                {...control}
                name="email"
                value={email}
                onChange={(event) => setEmail(event.target.value)}
                type="email"
                autoComplete="email"
                placeholder="name@example.com"
                required
                aria-invalid={credentialError || undefined}
              />
            )}
          </Field>
          <Field
            label="비밀번호"
            htmlFor="password"
            required
            describedBy={credentialError ? ERROR_ID : undefined}
          >
            {(control) => (
              <TextInput
                {...control}
                name="password"
                type="password"
                autoComplete="current-password"
                placeholder="비밀번호 입력"
                required
                aria-invalid={credentialError || undefined}
              />
            )}
          </Field>

          {errorText ? (
            <p className={styles.error} id={ERROR_ID} role="alert">
              {errorText}
            </p>
          ) : null}

          <Button type="submit" className={styles.submit}>
            <span>로그인</span>
            <span
              className={`${styles.assetIcon} ${styles.loginIcon}`}
              aria-hidden="true"
            />
          </Button>
        </ApiForm>

        {showPasskey || providers.length > 0 ? (
          <div className={styles.divider}>
            <span>또는</span>
          </div>
        ) : null}
        {showPasskey ? (
          <div className={styles.passkey}>
            {passkey.supported ? (
              <Button
                type="button"
                variant="secondary"
                className={styles.passkeyButton}
                disabled={passkey.phase === "waiting"}
                onClick={() => void passkey.submit(new FormData())}
              >
                <Icon
                  name={passkey.phase === "waiting" ? "pending" : "tokens"}
                />
                {passkey.phase === "waiting"
                  ? "기기에서 확인해 주세요"
                  : "패스키로 로그인"}
              </Button>
            ) : null}
            <p
              className={`${styles.passkeyStatus} ${passkey.phase === "failed" ? styles.error : ""}`}
              role="status"
            >
              {passkey.phase === "waiting" ? (
                <>
                  기기의 인증 창에서 로그인을 완료하세요.{" "}
                  <button type="button" onClick={passkey.cancel}>
                    취소
                  </button>
                </>
              ) : passkey.phase === "cancelled" ? (
                "로그인을 취소했습니다. 다시 시도하거나 비밀번호로 로그인하세요."
              ) : passkey.phase === "failed" ? (
                "패스키를 확인하지 못했습니다. 다시 시도해 주세요."
              ) : (
                "이메일이나 비밀번호를 입력하지 않아도 됩니다."
              )}
            </p>
          </div>
        ) : null}
        {passkeyAvailable === false ? (
          <p className={styles.unavailable}>
            이 접속에서는 패스키 로그인을 쓸 수 없습니다. 이유는 로그인한 뒤
            계정 페이지에서 볼 수 있습니다.
          </p>
        ) : null}
        {providers.length > 0 ? (
          <div className={styles.providers}>
            {providers.map((provider) => (
              <ApiForm
                key={provider.id}
                action={{ id: "login.oauthSignIn", args: [] }}
              >
                <input type="hidden" name="providerId" value={provider.id} />
                <button
                  type="submit"
                  className={buttonClassName({
                    variant: "secondary",
                    className: styles.providerButton,
                  })}
                >
                  <span
                    className={`${styles.assetIcon} ${styles.keyIcon}`}
                    aria-hidden="true"
                  />{" "}
                  {provider.name} 로 로그인
                </button>
              </ApiForm>
            ))}
          </div>
        ) : null}
      </div>
    </AuthFrame>
  );
}
