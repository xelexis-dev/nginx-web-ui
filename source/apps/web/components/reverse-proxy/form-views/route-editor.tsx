"use client";
import { useId } from "react";
import {
  Banner,
  Field,
  FieldGroup,
  SegmentedControl,
  Select,
  TextArea,
  TextInput,
} from "@repo/ui";
import type { RuntimeDnsStatus } from "@repo/contracts";
import { type DirectiveEditorState } from "@repo/presentation/nginx/directive-state";
import { routeRetryInfo } from "@repo/presentation/nginx/server-summary";
import type { LocationInput } from "../server-form-vm";
import { DirectiveEditor, FormDisclosure } from "../directive-editor";
import { RuntimeDnsOption } from "../runtime-dns";
import { runtimeDnsModeLabel } from "../runtime-dns-state";
import formStyles from "@/components/reverse-proxy/server-form.module.css";
export function RouteEditor({
  location,
  index,
  upstreams,
  runtimeDns,
  serverDirectives,
  cacheEnabled,
  gzipEnabled,
  error,
  onChange,
  absent,
  carried,
  nodeName,
  legacyNode = false,
}: {
  location: LocationInput;
  index: number;
  upstreams: { id: string; name: string; resolveAtRuntime?: boolean }[];
  runtimeDns?: RuntimeDnsStatus;
  serverDirectives: DirectiveEditorState;
  cacheEnabled: boolean;
  gzipEnabled: boolean;
  error?: string;
  onChange: (patch: Partial<LocationInput>) => void;
  absent?: boolean;
  carried?: boolean;
  nodeName?: string;
  legacyNode?: boolean;
}) {
  const group = useId();
  const route = location;
  const missing =
    route.mode === "upstream" &&
    (!!route.upstreamError ||
      (!!route.upstreamId &&
        !upstreams.some((u) => u.id === route.upstreamId)));
  return (
    <section
      className={formStyles.pfEditor}
      id={`route-editor-${index}`}
      aria-labelledby={`route-editor-heading-${index}`}
    >
      <p className={formStyles.directiveGroupNote}>
        이 경로로 들어온 요청에 적용됩니다.
      </p>
      {error ? <Banner tone="error" role="alert" title={error} /> : null}
      {missing ? (
        <Banner
          tone="error"
          role="alert"
          title="연결된 Upstream을 찾을 수 없습니다(삭제됐거나 이 서버 범위에 없음). 다른 Upstream을 고르거나 직접 주소로 바꾸세요."
        />
      ) : null}
      <div className={formStyles.pfFields}>
        <Field label="경로 (location path)" htmlFor={`route-path-${index}`}>
          <TextInput
            value={route.path}
            onChange={(e) => onChange({ path: e.target.value })}
            aria-label={`location ${index + 1} path`}
          />
        </Field>
        <FieldGroup id={`route-target-${index}`} legend="보낼 곳">
          <SegmentedControl
            label={`경로 ${index + 1} 보낼 곳`}
            value={route.mode}
            onChange={(mode) => onChange({ mode })}
            options={[
              { value: "upstream", label: "Upstream" },
              { value: "direct", label: "직접 주소" },
            ]}
          />
        </FieldGroup>
        <div data-wide>
          {route.mode === "upstream" ? (
            <Field
              label={nodeName ? `Upstream (${nodeName}의 목록)` : "Upstream"}
              htmlFor={`route-upstream-${index}`}
            >
              <Select
                value={missing ? "" : route.upstreamId}
                onChange={(upstreamId) => onChange({ upstreamId })}
                options={[
                  { value: "", label: "Upstream 선택" },
                  ...upstreams.map((u) => ({ value: u.id, label: u.name })),
                ]}
                aria-label={
                  nodeName
                    ? `Upstream (${nodeName}의 목록)`
                    : `location ${index + 1} upstream`
                }
              />
            </Field>
          ) : (
            <Field label="proxy_pass 주소" htmlFor={`route-proxy-${index}`}>
              <TextInput
                value={route.proxyPass}
                onChange={(e) => onChange({ proxyPass: e.target.value })}
                placeholder="http://127.0.0.1:3000"
                aria-label={`location ${index + 1} proxy_pass`}
              />
            </Field>
          )}
          {route.mode === "direct" ? (
            <RuntimeDnsOption
              checked={route.resolveAtRuntime === true}
              onChange={(resolveAtRuntime) => onChange({ resolveAtRuntime })}
              status={runtimeDns}
              label={`경로 ${index + 1}: 실행 중에 이름 확인`}
            />
          ) : (
            <p className={formStyles.directiveGroupNote}>
              이름 확인은 선택한 Upstream 설정을 따릅니다:{" "}
              {runtimeDnsModeLabel(
                upstreams.find((u) => u.id === route.upstreamId)
                  ?.resolveAtRuntime,
              )}
            </p>
          )}
        </div>
      </div>
      <FormDisclosure
        group={group}
        title="이 경로만 다르게"
        summary={[
          route.cacheEnabled === undefined
            ? `캐시: 서버 값 사용 (${cacheEnabled ? "켜기" : "끄기"})`
            : `캐시 ${route.cacheEnabled ? "켜기" : "끄기"}`,
          route.gzipEnabled === undefined
            ? `gzip: 서버 값 사용 (${gzipEnabled ? "켜기" : "끄기"})`
            : `gzip ${route.gzipEnabled ? "켜기" : "끄기"}`,
          route.ipAccess
            ? `IP ${route.ipAccess.mode === "whitelist" ? "허용" : "차단"} 목록`
            : "IP 서버 값 사용",
        ].join(" · ")}
      >
        <div className={formStyles.pfFields}>
          {(
            [
              ["cacheEnabled", "응답 캐시", cacheEnabled],
              ["gzipEnabled", "gzip 압축", gzipEnabled],
            ] as const
          ).map(([key, label, inherited]) => (
            <Field key={key} label={label} htmlFor={`route-${key}-${index}`}>
              <Select
                value={
                  route[key] === undefined
                    ? "inherit"
                    : route[key]
                      ? "on"
                      : "off"
                }
                onChange={(v) =>
                  onChange({ [key]: v === "inherit" ? undefined : v === "on" })
                }
                options={[
                  {
                    value: "inherit",
                    label: `서버 값 사용 (${inherited ? "켜기" : "끄기"})`,
                  },
                  { value: "on", label: "켜기" },
                  { value: "off", label: "끄기" },
                ]}
                aria-label={`location ${index + 1} ${key === "cacheEnabled" ? "캐시" : "gzip"}`}
              />
            </Field>
          ))}
          <Field label="이 경로 IP 접근 제어" htmlFor={`route-ip-${index}`}>
            <Select
              value={route.ipAccess?.mode ?? ""}
              onChange={(mode) =>
                onChange({
                  ipAccess: mode
                    ? {
                        mode: mode as "whitelist" | "blacklist",
                        entries: route.ipAccess?.entries ?? [],
                      }
                    : undefined,
                })
              }
              options={[
                { value: "", label: "서버 값 사용" },
                { value: "whitelist", label: "허용 목록" },
                { value: "blacklist", label: "차단 목록" },
              ]}
              aria-label={`location ${index + 1} IP 접근 제어 모드`}
            />
          </Field>
          {route.ipAccess ? (
            <Field
              label="IP / CIDR (한 줄에 하나)"
              htmlFor={`route-ip-entries-${index}`}
              error={
                route.ipAccess.mode === "whitelist" &&
                route.ipAccess.entries.every((s) => !s.trim())
                  ? "허용 목록이 비어 있으면 이 경로는 전원 차단됩니다 — 최소 1개를 입력하세요."
                  : undefined
              }
            >
              <TextArea
                value={route.ipAccess.entries.join("\n")}
                onChange={(e) =>
                  onChange({
                    ipAccess: {
                      mode: route.ipAccess!.mode,
                      entries: e.target.value.split(/\r?\n/),
                    },
                  })
                }
              />
            </Field>
          ) : null}
        </div>
      </FormDisclosure>
      {carried ? (
        <p className={formStyles.directiveGroupNote}>
          기존 디렉티브는 이 경로의 디렉티브를 고치지 않으면 저장된 순서·값
          그대로 저장됩니다. 고치면 편집기 순서로 다시 씁니다.
        </p>
      ) : null}
      <DirectiveEditor
        group={group}
        context="location"
        retryAvailable={!legacyNode}
        retryOtherInstances={route.retryOtherInstances === true}
        onRetryOtherInstancesChange={(enabled) =>
          onChange({ retryOtherInstances: enabled ? true : undefined })
        }
        retryInfo={routeRetryInfo(route, serverDirectives)}
        value={route.directives}
        onChange={(directives) => onChange({ directives })}
        inherited={serverDirectives}
        absent={absent}
      />
    </section>
  );
}
