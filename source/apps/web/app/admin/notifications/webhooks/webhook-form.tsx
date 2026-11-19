"use client";
import Link from "next/link";
import { Card, FormStack, FormBar } from "@repo/ui";
import adminStyles from "@/app/admin/admin-pages.module.css";
import resourceStyles from "@/components/resource-layout.module.css";
import { FormErrors } from "@/components/form-errors";
import { useActionState, useMemo, useState } from "react";
import {
  Banner,
  Switch,
  TextArea,
  TextInput,
  Button,
  buttonClassName,
  Select,
  Checkbox,
  Field,
  FieldGroup,
  CodePane,
  formFieldsClassName,
} from "@repo/ui";
import {
  DEFAULT_TEMPLATE,
  TEMPLATE_VARIABLES,
  WEBHOOK_EVENT_GROUPS,
  renderRequest,
  sampleEvent,
  type WebhookFormat,
} from "@repo/presentation/webhook";
import { useApiAction } from "@/lib/use-api-action";
import type { WebhookFormState } from "./form-types";
import styles from "../notifications.module.css";

const INITIAL: WebhookFormState = { ok: false };

/** actions.ts 의 KEEP_SECRET 과 같은 값 — 서버가 "저장된 값 유지"로 읽는 표식. */
const KEEP_SECRET = "__keep__";

/** 미리보기는 표본 사건으로 그린다. 고정 시각이라 렌더마다 값이 흔들리지 않는다. */
const PREVIEW_EVENT = sampleEvent(new Date("2026-08-10T00:00:00.000Z"));

/** 화면에서 고르는 종류. 저장 값(WebhookFormat)과 1:1 이 아니다 — 아래 storedFormat 참고. */
type FormatKind = "json" | "discord" | "dooray";

const FORMAT_OPTIONS = [
  { value: "json", label: "일반 (JSON)" },
  { value: "discord", label: "Discord" },
  { value: "dooray", label: "Dooray" },
];

const FORMAT_HINT: Record<FormatKind, string> = {
  json: "JSON 으로 POST 합니다. 본문을 비워두면 앱 기본값, 직접 쓰면 그 모양대로 보냅니다.",
  discord: "Discord 서버 설정 → 연동 → 웹후크에서 만든 URL 을 넣으세요.",
  dooray:
    "Dooray 대화방 설정 → 서비스 연동 → 인커밍 훅에서 만든 URL 을 넣으세요.",
};

const FORMAT_KINDS = new Set<FormatKind>(["json", "discord", "dooray"]);

export interface WebhookFormDefaults {
  id?: string;
  name: string;
  enabled: boolean;
  format: WebhookFormat;
  events: string[];
  bodyTemplate: string;
}

export interface WebhookFormProps {
  mode: "create" | "edit";
  defaults: WebhookFormDefaults;
  /** 저장된 URL 의 가림 표시. 원문은 내려보내지 않는다. */
  maskedUrl?: string | null;
  /** 저장된 헤더 이름들. 값은 내려보내지 않는다. */
  storedHeaderNames?: string[];
}

/** 서버 액션 하나의 결과 배너. 저장과 테스트가 각자의 배너를 갖는다(SMTP 폼과 같은 이유). */
function Result({ state, label }: { state: WebhookFormState; label: string }) {
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
 * 웹훅 수신처 생성·수정 폼(같은 컴포넌트, 액션만 다르다).
 *
 * 저장된 URL·헤더는 화면으로 되돌려주지 않는다 — 폼을 여는 것만으로 자격증명이
 * HTML 에 실려 나가면 봉인해서 저장한 의미가 없다. 비워두면 유지하고, 지우려면
 * 명시적으로 체크한다(SMTP 비밀번호와 같은 규칙).
 *
 * 모든 필드는 제어형이다: React 19 는 form action 이 끝나면 비제어 필드를
 * 리셋하므로, 검증에 걸려 되돌아왔을 때 입력이 통째로 사라진다.
 */
export function WebhookForm({
  mode,
  defaults,
  maskedUrl,
  storedHeaderNames = [],
}: WebhookFormProps) {
  const saveAction = useApiAction<WebhookFormState>({
    id: mode === "create" ? "admin.webhooks.create" : "admin.webhooks.update",
    args: [],
  });
  const [saveState, save, savePending] = useActionState(saveAction, INITIAL);
  const [testState, test, testPending] = useActionState(
    useApiAction<WebhookFormState>({ id: "admin.webhooks.test", args: [] }),
    INITIAL,
  );

  const [name, setName] = useState(defaults.name);
  const [enabled, setEnabled] = useState(defaults.enabled);
  // 화면에서는 수신처 종류만 고른다. 저장되는 format 은 JSON 을 골랐을 때만
  // 본문 유무에 따라 generic/custom 으로 갈린다(아래 storedFormat).
  const [kind, setKind] = useState<FormatKind>(
    defaults.format === "discord" || defaults.format === "dooray"
      ? defaults.format
      : "json",
  );
  const [url, setUrl] = useState("");
  const [headers, setHeaders] = useState("");
  const [clearHeaders, setClearHeaders] = useState(false);
  const [bodyTemplate, setBodyTemplate] = useState(defaults.bodyTemplate);
  const [events, setEvents] = useState<string[]>(defaults.events);

  const fieldErrors = saveState.fieldErrors ?? testState.fieldErrors;
  const pending = savePending || testPending;

  const hasStoredUrl = mode === "edit";
  // 비워두면 "유지" 표식을 보낸다. 새로 만들 때는 빈 값이 곧 "없음"이라 그대로 보낸다.
  const urlValue = url || (hasStoredUrl ? KEEP_SECRET : "");

  /**
   * 본문을 비워두면 `generic`(앱이 정한 고정 JSON), 뭔가 쓰면 `custom`(그 템플릿).
   * 운영자에게는 같은 선택지의 "기본이냐 손댔느냐"일 뿐이라 화면에서는 하나로 합치고,
   * 저장 값만 정확히 갈라 둔다 — 비워둔 쪽은 앱이 페이로드를 늘려도 따라간다.
   */
  const storedFormat: WebhookFormat =
    kind !== "json" ? kind : bodyTemplate.trim() ? "custom" : "generic";

  const preview = useMemo(() => {
    const rendered = renderRequest(storedFormat, bodyTemplate, PREVIEW_EVENT, {
      baseUrl: "https://nginx.example.com",
    });
    if (!rendered.ok) return rendered.error;
    try {
      return JSON.stringify(JSON.parse(rendered.request.body), null, 2);
    } catch {
      return rendered.request.body;
    }
  }, [storedFormat, bodyTemplate]);

  const toggleEvent = (type: string, on: boolean) => {
    setEvents((prev) =>
      on ? [...new Set([...prev, type])] : prev.filter((e) => e !== type),
    );
  };

  return (
    <form className={resourceStyles.form}>
      <FormErrors
        errors={fieldErrors}
        targets={{
          url: "wh-url",
          headers: "wh-headers",
          bodyTemplate: "wh-template",
          format: "wh-format",
          events: "wh-events",
        }}
      />
      <FormStack>
        {mode === "create" ? (
          <Banner
            tone="info"
            title={<>알림을 받을 HTTP 수신처를 추가합니다.</>}
          />
        ) : null}
        <Card
          id="admin-webhook"
          title={<>이 수신처로 보내기</>}
          description={<>꺼두면 설정은 남기고 보내지 않습니다.</>}
          actions={
            <Switch
              checked={enabled}
              onCheckedChange={setEnabled}
              label="사용 여부"
              visibleLabel="사용"
            />
          }
          errorCount={Object.values(fieldErrors ?? {}).filter(Boolean).length}
        >
          <div className={adminStyles.body}>
            {defaults.id ? (
              <input type="hidden" name="id" value={defaults.id} />
            ) : null}
            <input type="hidden" name="enabled" value={String(enabled)} />
            <input type="hidden" name="format" value={storedFormat} />
            <input type="hidden" name="url" value={urlValue} />
            <input
              type="hidden"
              name="clearHeaders"
              value={String(clearHeaders)}
            />
            {events.map((event) => (
              <input key={event} type="hidden" name="events" value={event} />
            ))}

            <div className={formFieldsClassName()}>
              <Field label="이름" htmlFor="wh-name" error={fieldErrors?.name}>
                <TextInput
                  name="name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="예: 인프라 Discord 채널"
                />
              </Field>

              <Field label="형식" htmlFor="wh-format" hint={FORMAT_HINT[kind]}>
                <Select
                  value={kind}
                  onChange={(v) =>
                    setKind(
                      FORMAT_KINDS.has(v as FormatKind)
                        ? (v as FormatKind)
                        : "json",
                    )
                  }
                  options={FORMAT_OPTIONS}
                  aria-label="형식"
                />
              </Field>

              <Field
                label="webhook URL"
                htmlFor="wh-url"
                hint={
                  hasStoredUrl
                    ? "비워두면 저장된 URL 을 그대로 유지합니다."
                    : "암호화해 저장하며 다시 보여주지 않습니다."
                }
                error={fieldErrors?.url}
                wide
              >
                <TextInput
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  autoComplete="off"
                  placeholder={
                    hasStoredUrl
                      ? `저장됨 (${maskedUrl ?? "•••"}) — 바꿀 때만 입력`
                      : "https://discord.com/api/webhooks/…"
                  }
                />
              </Field>

              <Field
                label="추가 헤더 (선택)"
                htmlFor="wh-headers"
                hint={
                  <>
                    한 줄에 하나씩{" "}
                    <span className={styles.hintCode}>이름: 값</span>. 값은
                    암호화해 저장합니다.
                  </>
                }
                error={fieldErrors?.headers}
                wide
              >
                {(control) => (
                  <>
                    <TextArea
                      {...control}
                      name="headers"
                      mono
                      value={headers}
                      onChange={(e) => setHeaders(e.target.value)}
                      disabled={clearHeaders}
                      placeholder={
                        storedHeaderNames.length > 0
                          ? `저장됨: ${storedHeaderNames.join(", ")} — 바꿀 때만 입력`
                          : "Authorization: Bearer xxxxx\nX-Source: nginx-web-ui"
                      }
                    />
                    {storedHeaderNames.length > 0 ? (
                      <Checkbox
                        label="저장된 헤더 지우기"
                        checked={clearHeaders}
                        onChange={(e) => setClearHeaders(e.target.checked)}
                      />
                    ) : null}
                  </>
                )}
              </Field>

              {kind === "json" ? (
                <Field
                  label="본문 (선택)"
                  htmlFor="wh-template"
                  hint={
                    <>
                      <p>
                        변수:{" "}
                        <span className={styles.hintCode}>
                          {TEMPLATE_VARIABLES.map((v) => `{{${v}}}`).join(" ")}
                        </span>
                      </p>
                      <p>
                        문자열은 큰따옴표 안에 넣으세요.{" "}
                        <span className={styles.hintCode}>
                          {"{{daysLeft}}"}
                        </span>
                        ·
                        <span className={styles.hintCode}>{"{{staging}}"}</span>{" "}
                        은 따옴표 없이,{" "}
                        <span className={styles.hintCode}>{"{{json}}"}</span> 은
                        전체 페이로드입니다.
                      </p>
                    </>
                  }
                  error={fieldErrors?.bodyTemplate}
                  wide
                >
                  <TextArea
                    name="bodyTemplate"
                    className={styles.templateArea}
                    mono
                    value={bodyTemplate}
                    onChange={(e) => setBodyTemplate(e.target.value)}
                    spellCheck={false}
                    placeholder={DEFAULT_TEMPLATE}
                  />
                </Field>
              ) : null}

              <FieldGroup
                id="wh-events"
                legend="보낼 사건"
                error={fieldErrors?.events}
                wide
              >
                {WEBHOOK_EVENT_GROUPS.map((group) => (
                  <div key={group.label} className={styles.eventGrid}>
                    {group.events.map((event) => (
                      <Checkbox
                        key={event.type}
                        tile
                        label={`${group.label} · ${event.label}`}
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

              {/* A read-only preview, not a field: the shared code pane, still announced as it changes. */}
              <div data-wide aria-live="polite">
                <CodePane
                  title="보내는 내용 미리보기"
                  info="만기 도래 알림 표본입니다. 테스트 전송도 이 내용을 보냅니다."
                  code={preview}
                  copyLabel={false}
                  headingLevel={3}
                />
              </div>
            </div>

            <Result state={saveState} label="저장" />
            <Result state={testState} label="테스트 전송" />
          </div>
        </Card>
        <FormBar>
          <Link
            href={
              defaults.id
                ? `/admin/notifications/webhooks/${defaults.id}`
                : "/admin/notifications/webhooks"
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
            {testPending ? "보내는 중..." : "테스트 전송"}
          </Button>
        </FormBar>
        <p className={styles.hint}>
          테스트 전송은 <strong>실제로 한 건</strong>이 수신처에 남습니다.
          저장하지 않은 값으로도 시험할 수 있습니다.
        </p>
      </FormStack>
    </form>
  );
}
