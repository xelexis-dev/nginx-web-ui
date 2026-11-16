"use client";
import Link from "next/link";
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
  buttonClassName,
  Checkbox,
  Field,
  FieldGroup,
  formFieldsClassName,
} from "@repo/ui";
import { WEBHOOK_EVENT_GROUPS } from "@repo/presentation/webhook";
import { useApiAction } from "@/lib/use-api-action";
import type { MailRecipientState } from "./form-types";
import styles from "../notifications.module.css";

const INITIAL: MailRecipientState = { ok: false };

export interface MailRecipientFormProps {
  mode: "create" | "edit";
  defaults: {
    id?: string;
    label: string;
    address: string;
    enabled: boolean;
    events: string[];
  };
  /** SMTP 가 준비돼 있는가 — 안 되어 있으면 저장은 되지만 메일이 나가지 않는다. */
  smtpReady: boolean;
  smtpProblem?: string | null;
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

/** 저장과 테스트가 각자의 배너를 갖는다 — 하나로 합치면 앞선 성공이 뒤의 실패를 가린다. */
function Result({
  state,
  label,
}: {
  state: MailRecipientState;
  label: string;
}) {
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
 * 메일 수신처 생성·수정 폼(같은 컴포넌트, 액션만 다르다). webhook 수신처 폼과 같은
 * 모양이고, 다른 점은 "받는 곳"이 URL 이 아니라 주소라는 것뿐이다.
 *
 * 모든 필드는 제어형이다: React 19 는 form action 이 끝나면 비제어 필드를 리셋한다.
 */
export function MailRecipientForm({
  mode,
  defaults,
  smtpReady,
  smtpProblem,
}: MailRecipientFormProps) {
  const saveAction = useApiAction<MailRecipientState>({
    id:
      mode === "create"
        ? "admin.mailRecipients.create"
        : "admin.mailRecipients.update",
    args: [],
  });
  const [saveState, save, savePending] = useActionState(saveAction, INITIAL);
  const [testState, test, testPending] = useActionState(
    useApiAction<MailRecipientState>({
      id: "admin.mailRecipients.test",
      args: [],
    }),
    INITIAL,
  );

  const [label, setLabel] = useState(defaults.label);
  const [address, setAddress] = useState(defaults.address);
  const [enabled, setEnabled] = useState(defaults.enabled);
  const [events, setEvents] = useState<string[]>(defaults.events);

  const fieldErrors = saveState.fieldErrors ?? testState.fieldErrors;
  const pending = savePending || testPending;

  const toggleEvent = (type: string, on: boolean) => {
    setEvents((prev) =>
      on ? [...new Set([...prev, type])] : prev.filter((e) => e !== type),
    );
  };

  return (
    <form className={resourceStyles.form}>
      <FormErrors errors={fieldErrors} targets={{ events: "mr-events" }} />
      <FormStack>
        {mode === "create" ? (
          <Banner
            tone="info"
            title={<>인증서 알림을 받을 주소를 등록합니다.</>}
          />
        ) : null}
        <Card
          id="admin-recipient"
          title={<>이 주소로 보내기</>}
          description={<>꺼두면 설정은 남기고 보내지 않습니다.</>}
          actions={
            <Toggle
              value={enabled}
              onChange={setEnabled}
              ariaLabel="사용 여부"
              offLabel="사용 안 함"
              onLabel="사용"
            />
          }
          errorCount={Object.values(fieldErrors ?? {}).filter(Boolean).length}
        >
          <div className={adminStyles.body}>
            {defaults.id ? (
              <input type="hidden" name="id" value={defaults.id} />
            ) : null}
            <input type="hidden" name="enabled" value={String(enabled)} />
            {events.map((event) => (
              <input key={event} type="hidden" name="events" value={event} />
            ))}

            {!smtpReady ? (
              <Banner
                tone={"error"}
                title={
                  <>
                    SMTP 가 준비되지 않아 메일이 나가지 않습니다
                    {smtpProblem ? ` — ${smtpProblem}` : ""}
                  </>
                }
                role="status"
              >
                <p className={styles.hint}>
                  사이트 설정 → SMTP 설정을 마치면 그대로 동작합니다.
                </p>
              </Banner>
            ) : null}

            <div className={formFieldsClassName()}>
              <Field
                label="수신 주소"
                htmlFor="mr-address"
                hint="주소마다 따로 발송해 수신자끼리 서로를 보지 않습니다."
                error={fieldErrors?.address}
              >
                <TextInput
                  name="address"
                  value={address}
                  onChange={(e) => setAddress(e.target.value)}
                  placeholder="ops@example.com"
                  autoComplete="off"
                />
              </Field>

              <Field
                label="표시 이름 (선택)"
                htmlFor="mr-label"
                hint="비우면 주소를 그대로 표시합니다."
              >
                <TextInput
                  name="label"
                  value={label}
                  onChange={(e) => setLabel(e.target.value)}
                  placeholder="예: 인프라 운영팀"
                />
              </Field>

              <FieldGroup
                id="mr-events"
                legend="받을 사건"
                error={fieldErrors?.events}
                wide
              >
                {WEBHOOK_EVENT_GROUPS.map((group) => (
                  <div key={group.label} className={styles.eventGrid}>
                    {group.events.map((event) => (
                      <Checkbox
                        key={event.type}
                        className={styles.eventItem}
                        label={
                          <span className={styles.eventLabel}>
                            {group.label} · {event.label}
                          </span>
                        }
                        description={event.hint || undefined}
                        checked={events.includes(event.type)}
                        onChange={(e) =>
                          toggleEvent(event.type, e.target.checked)
                        }
                      />
                    ))}
                  </div>
                ))}
              </FieldGroup>
            </div>

            <Result state={saveState} label="저장" />
            <Result state={testState} label="테스트 발송" />
          </div>
        </Card>
        <FormBar>
          <Link
            href={
              defaults.id
                ? `/admin/notifications/mail/${defaults.id}`
                : "/admin/notifications/mail"
            }
            className={buttonClassName({ variant: "secondary" })}
          >
            취소
          </Link>
          <Button type="submit" formAction={save} disabled={pending}>
            {savePending ? "저장 중..." : "저장"}
          </Button>
          <Button
            type="submit"
            formAction={test}
            disabled={pending}
            variant="secondary"
          >
            {testPending ? "보내는 중..." : "테스트 발송"}
          </Button>
        </FormBar>
        <p className={styles.hint}>
          테스트 발송은 <strong>실제로 한 통</strong>을 보냅니다. 저장하지 않은
          주소로도 시험할 수 있습니다.
        </p>
      </FormStack>
    </form>
  );
}
