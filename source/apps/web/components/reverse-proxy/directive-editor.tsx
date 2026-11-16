"use client";

import {
  Icon,
  Button,
  TextArea,
  TextInput,
  Field as UiField,
  Switch,
  Banner,
} from "@repo/ui";

import { useId, type ReactElement, type ReactNode } from "react";
import { Combobox, Select } from "@repo/ui";
import type { DirectiveContext } from "@repo/presentation/nginx";
import {
  REQUEST_HEADER_CATALOG,
  RESPONSE_HEADER_CATALOG,
  suggestedValues,
  type HeaderCatalog,
} from "@repo/presentation/nginx/header-catalog";
import type {
  DirectiveEditorState,
  HeaderRow,
} from "@repo/presentation/nginx/directive-state";
import type { RetryInfo } from "@repo/presentation/nginx/server-summary";
import { WebSocketPresetControl } from "./websocket-preset-control";
import formStyles from "@/components/reverse-proxy/server-form.module.css";
import styles from "@/app/(app)/reverse-proxy/reverse-proxy.module.css";

interface DirectiveEditorProps {
  context: DirectiveContext;
  value: DirectiveEditorState;
  onChange: (next: DirectiveEditorState) => void;
  inherited?: DirectiveEditorState;
  absent?: boolean;
  group?: string;
  retryOtherInstances?: boolean;
  retryAvailable?: boolean;
  onRetryOtherInstancesChange?: (enabled: boolean) => void;
  retryInfo?: RetryInfo;
}

const HTTP_VERSION_OPTIONS = [
  { value: "", label: "(미설정)" },
  { value: "1.0", label: "1.0" },
  { value: "1.1", label: "1.1" },
];

const BUFFERING_OPTIONS = [
  { value: "", label: "(미설정)" },
  { value: "on", label: "on" },
  { value: "off", label: "off" },
];

const REWRITE_FLAG_OPTIONS = [
  { value: "", label: "(없음)" },
  { value: "break", label: "break" },
  { value: "last", label: "last" },
  { value: "redirect", label: "redirect" },
  { value: "permanent", label: "permanent" },
];

const LOCATION_ADVANCED_HINT =
  "예: proxy_redirect off;  ·  return 301 https://$host$request_uri;  ·  expires 30d;";
const SERVER_ADVANCED_HINT =
  "예: server_tokens off;  ·  keepalive_timeout 65s;  ·  gzip on;";

/** Shared disclosures keep the current values visible before expanding an editor. */
export function FormDisclosure({
  title,
  summary,
  children,
  group,
}: {
  title: string;
  summary: string;
  children: ReactNode;
  group?: string;
}) {
  return (
    <details className={formStyles.pfDisclosure} name={group}>
      <summary>
        <span>{title}</span>
        <span className={formStyles.pfClosedValue}>{summary}</span>
        <span className={formStyles.pfDisclosureMark} aria-hidden="true">
          <Icon name="chevronDown" size={14} />
        </span>
      </summary>
      <div className={formStyles.pfDisclosureBody}>{children}</div>
    </details>
  );
}
const headerNames = (rows: HeaderRow[]) =>
  rows
    .map((row) => row.name.trim())
    .filter(Boolean)
    .join(", ") || "없음";
export function DirectiveEditor({
  context,
  value,
  onChange,
  inherited,
  absent = false,
  group: suppliedGroup,
  retryOtherInstances = false,
  retryAvailable = true,
  onRetryOtherInstancesChange,
  retryInfo,
}: DirectiveEditorProps) {
  const ownGroup = useId();
  const group = suppliedGroup ?? ownGroup;
  const isLocation = context === "location";
  const patch = (next: Partial<DirectiveEditorState>) =>
    onChange({ ...value, ...next });
  const timeValue = (key: keyof DirectiveEditorState["timeouts"]) =>
    isLocation && key === "connect" && retryInfo
      ? retryInfo.connectDisplay
      : value.timeouts[key] ||
        (inherited?.timeouts[key]
          ? `서버 값 사용 (${inherited.timeouts[key]})`
          : "nginx 기본값");
  const advanced = value.advanced
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"));
  const advancedNames = [
    ...new Set(advanced.map((line) => line.split(/\s/)[0])),
  ].join(", ");
  return (
    <div className={formStyles.directiveEditor}>
      {isLocation ? (
        <FormDisclosure
          group={group}
          title="연결 방식 · 요청 헤더"
          summary={[
            retryOtherInstances ? retryInfo?.summary : "추가 재시도: 끄기",
            absent
              ? "기존 기본값 (디렉티브 없음)"
              : `HTTP ${value.httpVersion || "기본값"} · buffering ${value.buffering || "기본값"} · ${headerNames(value.headers)}`,
          ]
            .filter(Boolean)
            .join(" · ")}
        >
          <UiField
            label="재시작 중 다른 인스턴스로 재시도"
            htmlFor={`${ownGroup}-retry`}
            info="3초는 다음 시도를 시작할 수 있는 시간이며 전체 요청 제한이 아닙니다."
            hint={
              <>
                {!retryAvailable ? <>1.19 노드에서는 사용할 수 없습니다. 노드를 2.0으로 업데이트하세요.<br /></> : null}
                정상 인스턴스가 2개 이상 있어야 효과가 있습니다. 연결 실패나
                502·503·504 응답이면 다른 인스턴스로 한 번 더 보냅니다.
                <br />
                연결 대기 제한: {retryInfo?.connectDisplay ?? "nginx 기본값"}.
                연결 후 응답 대기는 읽기 제한(현재 {retryInfo?.read ?? "60s"})을
                따릅니다.
                <br />이 설정을 켠 상태에서 연결은 됐지만 응답하지 않는 대상은
                읽기 제한까지 기다립니다. 읽기 대기로 재시도 시작 한도 3초가
                지난 경우에는 다른 인스턴스로 다시 보내지 않습니다.
                {retryInfo?.replacedServer ? (
                  <>
                    <br />
                    서버 값 {retryInfo.replacedServer} 대신 2초를 적용합니다.
                  </>
                ) : null}
                <br />
                이미 보낸 POST·PATCH 요청은 다시 보내지 않습니다.
                <br />
                대상이 하나뿐이면 다른 대상으로 재시도할 수 없지만 연결 제한은
                적용됩니다.
                {!retryOtherInstances ? (
                  <>
                    <br />
                    꺼져 있으면 추가 재시도 정책을 쓰지 않습니다 (nginx 기본
                    재시도는 유지).
                  </>
                ) : null}
              </>
            }
          >
            <Switch
              checked={retryOtherInstances}
              disabled={!retryAvailable}
              onCheckedChange={(enabled) =>
                onRetryOtherInstancesChange?.(enabled)
              }
              label="재시작 중 다른 인스턴스로 재시도"
            />
          </UiField>
          {retryInfo?.warning ? (
            <Banner tone="warning" role="status" title={retryInfo.warning} />
          ) : null}
          {absent ? (
            <p className={formStyles.directiveGroupNote}>
              저장된 디렉티브 없음 — nginx가 기본값으로 Host $host만 보냅니다.
            </p>
          ) : null}
          <WebSocketPresetControl value={value} onChange={onChange} />
          <div className={formStyles.gridFields}>
            <Field label="proxy_http_version">
              <Select
                value={value.httpVersion}
                onChange={(httpVersion) => patch({ httpVersion })}
                options={HTTP_VERSION_OPTIONS}
                aria-label="proxy_http_version"
              />
            </Field>
            <Field label="proxy_buffering">
              <Select
                value={value.buffering}
                onChange={(buffering) => patch({ buffering })}
                options={BUFFERING_OPTIONS}
                aria-label="proxy_buffering"
              />
            </Field>
          </div>
          <KeyValueSection
            title="요청 헤더 (proxy_set_header)"
            rows={value.headers}
            catalog={REQUEST_HEADER_CATALOG}
            namePlaceholder="Host"
            valuePlaceholder="$host"
            onChange={(headers) => patch({ headers })}
          />
        </FormDisclosure>
      ) : null}
      <FormDisclosure
        group={group}
        title="시간 제한 · 본문 크기"
        summary={`읽기 ${timeValue("read")} · 전송 ${timeValue("send")} · 연결 ${timeValue("connect")} · 본문 ${value.bodySize || (inherited?.bodySize ? `서버 값 사용 (${inherited.bodySize})` : "nginx 기본값")}`}
      >
        <div className={formStyles.gridFields}>
          {(
            [
              ["read", "proxy_read_timeout", "60s"],
              ["send", "proxy_send_timeout", "60s"],
              ["connect", "proxy_connect_timeout", "10s"],
            ] as const
          ).map(([key, label, fallback]) => (
            <Field key={key} label={label}>
              <TextInput
                value={value.timeouts[key]}
                onChange={(event) =>
                  patch({
                    timeouts: { ...value.timeouts, [key]: event.target.value },
                  })
                }
                placeholder={
                  key === "connect" && isLocation && retryInfo
                    ? retryInfo.connect || "nginx 기본값"
                    : key === "read" && retryInfo
                      ? retryInfo.read
                      : inherited?.timeouts[key] || fallback
                }
                aria-label={label}
              />
            </Field>
          ))}
          <Field label="client_max_body_size">
            <TextInput
              value={value.bodySize}
              onChange={(event) => patch({ bodySize: event.target.value })}
              placeholder={inherited?.bodySize || "10m"}
              aria-label="client_max_body_size"
            />
          </Field>
        </div>
        {isLocation ? (
          <p className={formStyles.directiveGroupNote}>
            비워 두면 서버 값 또는 nginx 기본값을 사용합니다.
          </p>
        ) : null}
      </FormDisclosure>
      <FormDisclosure
        group={group}
        title="응답 헤더"
        summary={headerNames(value.addHeaders)}
      >
        <KeyValueSection
          title="응답 헤더 (add_header)"
          rows={value.addHeaders}
          catalog={RESPONSE_HEADER_CATALOG}
          namePlaceholder="X-Frame-Options"
          valuePlaceholder="SAMEORIGIN"
          onChange={(addHeaders) => patch({ addHeaders })}
        />
      </FormDisclosure>
      {isLocation ? (
        <FormDisclosure
          group={group}
          title="rewrite"
          summary={
            value.rewrites
              .map((row) => row.from)
              .filter(Boolean)
              .join(", ") || "없음"
          }
        >
          <div className={styles.kvRows}>
            {value.rewrites.map((row, index) => (
              <div key={index} className={formStyles.rewriteRow}>
                <TextInput
                  value={row.from}
                  onChange={(event) =>
                    updateRow(
                      value.rewrites,
                      index,
                      { from: event.target.value },
                      (rewrites) => patch({ rewrites }),
                    )
                  }
                  placeholder="^/old/(.*)$"
                  aria-label="rewrite 정규식"
                />
                <TextInput
                  value={row.to}
                  onChange={(event) =>
                    updateRow(
                      value.rewrites,
                      index,
                      { to: event.target.value },
                      (rewrites) => patch({ rewrites }),
                    )
                  }
                  placeholder="/new/$1"
                  aria-label="rewrite 대체"
                />
                <Select
                  value={row.flag}
                  onChange={(flag) =>
                    updateRow(value.rewrites, index, { flag }, (rewrites) =>
                      patch({ rewrites }),
                    )
                  }
                  options={REWRITE_FLAG_OPTIONS}
                  aria-label="rewrite 플래그"
                />
                <Button
                  type="button"
                  variant="dangerQuiet"
                  aria-label="rewrite 삭제"
                  onClick={() =>
                    patch({
                      rewrites: value.rewrites.filter((_, i) => i !== index),
                    })
                  }
                >
                  삭제
                </Button>
              </div>
            ))}
            <Button
              type="button"
              variant="secondary"
              onClick={() =>
                patch({
                  rewrites: [...value.rewrites, { from: "", to: "", flag: "" }],
                })
              }
            >
              <Icon name="plus" />
              rewrite 추가
            </Button>
          </div>
        </FormDisclosure>
      ) : null}
      <FormDisclosure
        group={group}
        title="고급 — 추가 디렉티브 직접 입력"
        summary={
          advanced.length ? `${advanced.length}줄 · ${advancedNames}` : "없음"
        }
      >
        <p className={formStyles.directiveGroupNote}>
          한 줄에 하나. 허용된 디렉티브만 사용할 수 있습니다.
          <br />
          {isLocation ? LOCATION_ADVANCED_HINT : SERVER_ADVANCED_HINT}
        </p>
        <TextArea
          mono
          id={isLocation ? undefined : "server-directives-advanced"}
          value={value.advanced}
          onChange={(event) => patch({ advanced: event.target.value })}
          placeholder={
            isLocation ? "proxy_redirect off;" : "server_tokens off;"
          }
          aria-label="추가 디렉티브"
          spellCheck={false}
        />
      </FormDisclosure>
    </div>
  );
}

// A labelled scalar field: the shared Field with a generated id. Field puts a real
// <label htmlFor> beside the control and gives the control that id — clicking the
// label focuses the input / opens the Select, and it names the control for screen
// readers. NOT a <label> wrapping the control: the Select renders a popover of
// option buttons, and an option click would bubble to a wrapping label and
// re-toggle the trigger.
function Field({
  label,
  children,
}: {
  label: string;
  children: ReactElement<Record<string, unknown>>;
}) {
  const id = useId();
  return (
    <UiField label={label} htmlFor={id}>
      {children}
    </UiField>
  );
}

function updateRow<T>(
  rows: T[],
  index: number,
  patch: Partial<T>,
  commit: (next: T[]) => void,
) {
  commit(rows.map((row, i) => (i === index ? { ...row, ...patch } : row)));
}

/**
 * Repeatable header key/value rows (proxy_set_header, add_header).
 *
 * Both cells are `Combobox`, not `Select`: the catalog covers what people
 * actually set behind a reverse proxy, but a custom header must stay a normal
 * thing to type. The value cell's suggestions are derived from the name on the
 * SAME row, so picking `X-Forwarded-For` offers `$proxy_add_x_forwarded_for`
 * while an unknown name falls back to the common nginx variables.
 */
function KeyValueSection({
  title,
  rows,
  catalog,
  namePlaceholder,
  valuePlaceholder,
  onChange,
}: {
  title: string;
  rows: HeaderRow[];
  catalog: HeaderCatalog;
  namePlaceholder: string;
  valuePlaceholder: string;
  onChange: (rows: HeaderRow[]) => void;
}) {
  const nameOptions = catalog.headers.map((h) => ({
    value: h.name,
    hint: h.hint,
  }));

  return (
    <div className={formStyles.directiveSection}>
      <span className={formStyles.directiveSectionTitle}>{title}</span>
      <div className={styles.kvRows}>
        {rows.map((h, i) => (
          <div key={i} className={styles.kvRow}>
            <Combobox
              options={nameOptions}
              value={h.name}
              onChange={(name) => updateRow(rows, i, { name }, onChange)}
              placeholder={namePlaceholder}
              aria-label="헤더 이름"
            />
            <Combobox
              options={suggestedValues(catalog, h.name).map((v) => ({
                value: v,
              }))}
              value={h.value}
              onChange={(value) => updateRow(rows, i, { value }, onChange)}
              placeholder={valuePlaceholder}
              aria-label="헤더 값"
            />
            <Button
              type="button"
              variant="dangerQuiet"
              onClick={() => onChange(rows.filter((_, j) => j !== i))}
              aria-label="헤더 삭제"
            >
              삭제
            </Button>
          </div>
        ))}
        <Button
          type="button"
          variant="secondary"
          onClick={() => onChange([...rows, { name: "", value: "" }])}
        >
          <Icon name="plus" />
          헤더 추가
        </Button>
      </div>
    </div>
  );
}
