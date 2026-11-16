"use client";
import { Card, FormStack, FormBar } from "@repo/ui";
import adminStyles from "@/app/admin/admin-pages.module.css";
import resourceStyles from "@/components/resource-layout.module.css";
import { FormErrors } from "@/components/form-errors";
import { useActionState, useState } from "react";
import {
  Banner,
  SegmentedControl,
  TextInput,
  Button,
  Select,
  Checkbox,
  Field,
  formFieldsClassName,
} from "@repo/ui";
import { useApiAction } from "@/lib/use-api-action";
import type { SmtpState } from "./form-types";

const INITIAL: SmtpState = { ok: false };

/** API 가 "기존 비밀번호 유지"로 해석하는 표식. */
const KEEP_PASSWORD = "__keep__";

export interface SmtpFormProps {
  defaults: {
    enabled: boolean;
    host: string;
    port: number;
    secure: boolean;
    username: string;
    fromEmail: string;
    fromName: string;
    rejectUnauthorized: boolean;
  };
  /** 비밀번호가 저장되어 있는지 — 원문은 절대 내려보내지 않는다. */
  hasStoredPassword: boolean;
}

function Toggle({
  value,
  onChange,
  ariaLabel,
  offLabel,
  onLabel,
}: {
  value: boolean;
  onChange: (v: boolean) => void;
  ariaLabel: string;
  offLabel: string;
  onLabel: string;
}) {
  return (
    <SegmentedControl
      label={ariaLabel}
      value={value}
      onChange={onChange}
      options={[
        { value: false, label: offLabel },
        { value: true, label: onLabel },
      ]}
    />
  );
}

/**
 * 요청 하나의 결과 배너. 저장과 테스트가 각자의 배너를 갖는다 — 하나로 합치면
 * 직전 "테스트 성공"이 뒤이은 "저장 실패"를 가려 아무 일도 안 일어난 것처럼 보인다.
 */
function Result({ state, label }: { state: SmtpState; label: string }) {
  if (state.error) {
    return (
      <Banner
        tone={"error"}
        title={
          <>
            {label} 실패 — {state.error}
          </>
        }
        role="alert"
      ></Banner>
    );
  }
  if (state.ok && state.message) {
    return (
      <Banner
        tone={"success"}
        title={<>{state.message}</>}
        role="status"
      ></Banner>
    );
  }
  return null;
}

/**
 * SMTP 설정 폼.
 *
 * 저장된 비밀번호는 화면으로 되돌려주지 않는다 — 폼을 여는 것만으로 자격증명이
 * HTML 에 실려 나가면 저장 시 암호화한 의미가 없다. 대신 비워두면 기존 값을
 * 유지하고(hidden 표식), 지우려면 "비밀번호 지우기"를 켠다.
 *
 * 저장과 테스트가 같은 필드를 쓰므로 두 API 요청을 하나의 form 에 붙이고
 * formAction 으로 분기한다 — 테스트할 때 굳이 먼저 저장하지 않아도 된다.
 */
export function SmtpForm({ defaults, hasStoredPassword }: SmtpFormProps) {
  const [saveState, saveAction, savePending] = useActionState(
    useApiAction<SmtpState>({ id: "admin.smtp.update", args: [] }),
    INITIAL,
  );
  const [testState, testAction, testPending] = useActionState(
    useApiAction<SmtpState>({ id: "admin.smtp.test", args: [] }),
    INITIAL,
  );

  const [enabled, setEnabled] = useState(defaults.enabled);
  const [secure, setSecure] = useState(defaults.secure);
  const [rejectUnauthorized, setRejectUnauthorized] = useState(
    defaults.rejectUnauthorized,
  );
  const [password, setPassword] = useState("");
  const [clearPassword, setClearPassword] = useState(false);
  // 텍스트 필드도 전부 제어형이다: React 19는 form action이 끝나면 폼의 비제어
  // 필드를 리셋하므로, 검증에 걸려 되돌아왔을 때 입력이 통째로 사라진다.
  const [host, setHost] = useState(defaults.host);
  const [port, setPort] = useState(String(defaults.port));
  const [username, setUsername] = useState(defaults.username);
  const [fromEmail, setFromEmail] = useState(defaults.fromEmail);
  const [fromName, setFromName] = useState(defaults.fromName);
  const [testTo, setTestTo] = useState("");

  // 저장과 테스트는 각자의 결과를 낸다. 하나로 합치면 "테스트 성공" 배너가
  // 그 뒤의 저장 실패를 가려버린다(실제로 겪은 문제).
  const fieldErrors = saveState.fieldErrors ?? testState.fieldErrors;
  const pending = savePending || testPending;

  // 비었고 지우기도 아니면 "유지" 표식을 보낸다. 저장된 게 없으면 빈 문자열이 곧 "없음".
  const passwordValue = clearPassword
    ? ""
    : password || (hasStoredPassword ? KEEP_PASSWORD : "");

  return (
    <form className={resourceStyles.form}>
      <FormErrors
        errors={fieldErrors}
        targets={{
          password: "smtp-password",
          fromEmail: "smtp-from-email",
          testTo: "smtp-test-to",
        }}
      />
      <FormStack>
        <Card
          id="admin-smtp"
          title={<>메일 발송</>}
          description={<>꺼두면 설정은 남기고 메일을 보내지 않습니다.</>}
          actions={
            <Toggle
              value={enabled}
              onChange={setEnabled}
              ariaLabel="메일 발송"
              offLabel="사용 안 함"
              onLabel="사용"
            />
          }
          errorCount={Object.values(fieldErrors ?? {}).filter(Boolean).length}
        >
          <div className={adminStyles.body}>
            <input type="hidden" name="enabled" value={String(enabled)} />
            <input type="hidden" name="secure" value={String(secure)} />
            <input
              type="hidden"
              name="rejectUnauthorized"
              value={String(rejectUnauthorized)}
            />
            <input type="hidden" name="password" value={passwordValue} />

            <div className={formFieldsClassName()}>
              <Field
                label="SMTP 호스트"
                htmlFor="smtp-host"
                error={fieldErrors?.host}
              >
                <TextInput
                  name="host"
                  value={host}
                  onChange={(e) => setHost(e.target.value)}
                  placeholder="smtp.example.com"
                />
              </Field>

              <Field label="포트" htmlFor="smtp-port" error={fieldErrors?.port}>
                <TextInput
                  name="port"
                  type="number"
                  min={1}
                  max={65535}
                  value={port}
                  onChange={(e) => setPort(e.target.value)}
                />
              </Field>

              <Field
                label="사용자명"
                htmlFor="smtp-username"
                hint="비우면 인증 없이 접속합니다(사내 릴레이 등)."
              >
                <TextInput
                  name="username"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  autoComplete="off"
                />
              </Field>

              <Field
                label="비밀번호"
                htmlFor="smtp-password"
                hint="비우면 저장된 비밀번호를 그대로 유지합니다."
              >
                {(control) => (
                  <>
                    <TextInput
                      {...control}
                      type="password"
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      disabled={clearPassword}
                      autoComplete="new-password"
                      placeholder={
                        hasStoredPassword ? "저장됨 — 바꿀 때만 입력" : "미설정"
                      }
                    />
                    {hasStoredPassword ? (
                      <Checkbox
                        label="저장된 비밀번호 지우기"
                        checked={clearPassword}
                        onChange={(e) => setClearPassword(e.target.checked)}
                      />
                    ) : null}
                  </>
                )}
              </Field>

              <Field
                label="발신 주소"
                htmlFor="smtp-from-email"
                error={fieldErrors?.fromEmail}
              >
                <TextInput
                  name="fromEmail"
                  value={fromEmail}
                  onChange={(e) => setFromEmail(e.target.value)}
                  placeholder="nginx-web-ui@example.com"
                />
              </Field>

              <Field label="발신자 이름" htmlFor="smtp-from-name">
                <TextInput
                  name="fromName"
                  value={fromName}
                  onChange={(e) => setFromName(e.target.value)}
                  placeholder="nginx-web-ui"
                />
              </Field>

              <Field label="연결 방식" htmlFor="smtp-secure">
                <Select
                  value={secure ? "tls" : "starttls"}
                  onChange={(v) => setSecure(v === "tls")}
                  options={[
                    { value: "starttls", label: "STARTTLS (보통 587)" },
                    { value: "tls", label: "즉시 TLS (보통 465)" },
                  ]}
                  aria-label="연결 방식"
                />
              </Field>

              <Field label="서버 인증서 검증" htmlFor="smtp-reject">
                <Select
                  value={rejectUnauthorized ? "on" : "off"}
                  onChange={(v) => setRejectUnauthorized(v === "on")}
                  options={[
                    { value: "on", label: "검증함 (권장)" },
                    { value: "off", label: "검증 안 함 (사설 인증서)" },
                  ]}
                  aria-label="서버 인증서 검증"
                />
              </Field>

              <Field
                label="테스트 수신 주소"
                htmlFor="smtp-test-to"
                hint="비우면 연결·인증까지만 확인하고, 주소를 넣으면 실제로 한 통 보냅니다."
                wide
              >
                <TextInput
                  name="testTo"
                  value={testTo}
                  onChange={(e) => setTestTo(e.target.value)}
                  placeholder="me@example.com"
                />
              </Field>
            </div>

            <Result state={saveState} label="저장" />
            <Result state={testState} label="연결 확인" />
          </div>
        </Card>
        <FormBar>
          <Button type="submit" formAction={saveAction} disabled={pending}>
            {savePending ? "저장 중..." : "저장"}
          </Button>
          <Button
            type="submit"
            formAction={testAction}
            disabled={pending}
            variant="secondary"
          >
            {testPending ? "확인 중..." : "연결 확인 / 테스트 발송"}
          </Button>
        </FormBar>
      </FormStack>
    </form>
  );
}
