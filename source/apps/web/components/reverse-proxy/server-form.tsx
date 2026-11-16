"use client";
import { UNAPPLIED_CONFIG_INFO } from "@/components/reverse-proxy/save-info";
import { useActionState, useEffect, useRef, useState } from "react";
import type { RuntimeDnsStatus } from "@repo/contracts";
import { Banner, FormStack } from "@repo/ui";
import { certificateCoversHostname } from "@repo/presentation/certificate-hostname";
import type {
  CacheConfig,
  GzipConfig,
  RealIpConfig,
  RealIpHeader,
  TlsTuning,
} from "@repo/presentation/nginx";
import {
  emptyDirectiveState,
  type DirectiveEditorState,
} from "@repo/presentation/nginx/directive-state";
import { NodeErrorCard } from "@/components/node-error";
import { useToast } from "@/components/notifications/toast-provider";
import type { NodeActionState, NodeFormAction } from "@/lib/node-action-state";
import { useApiAction } from "@/lib/use-api-action";
import { newLocationInput, serializeLocations } from "./server-form-vm";
import {
  carryDirectives,
  directiveStateUnchanged,
} from "@repo/presentation/nginx/directive-carry";
import { serverSummary } from "@repo/presentation/nginx/server-summary";
import type {
  CertificateOption,
  LocationInput,
  ServerFormValues,
} from "./server-form-vm";
import { RuntimeDnsNotice } from "./runtime-dns";
import { IdentityStrip } from "./form-views/identity-strip";
import { RoutesView, locationError } from "./form-views/routes-view";
import { RouteEditor } from "./form-views/route-editor";
import { ProtectView } from "./form-views/protect-view";
import { CommonView } from "./form-views/common-view";
import { SummaryRail, type SummaryItem } from "./summary-rail";
import { useProxyFormGuard } from "./use-proxy-form-guard";
import type { OperationsTarget } from "@repo/presentation/operations-target";
import layout from "@/components/page-layout.module.css";
import formStyles from "@/components/reverse-proxy/server-form.module.css";
const INITIAL: NodeActionState = { ok: false };
interface ServerFormProps {
  /**
   * 로컬 액션도 노드 액션도 그대로 들어간다 — `NodeFormAction` 이 기존
   * `ActionState` 액션의 상위 계약이다(그 타입의 문서). **`ActionState` 로
   * 되돌리지 마라**: 추가 필드가 전부 optional 이라 컴파일은 그대로 통과하는데
   * 아래 `state.nodeError` 가 타입에서 사라져 §9 카드가 영영 안 그려진다.
   */
  action: NodeFormAction;
  /**
   * upstream 후보. **그 서버와 같은 출처여야 한다** — 노드 화면에 게이트웨이
   * 자신의 목록을 실으면 저장은 되고 노드에서 참조가 깨진다(노드의
   * `buildServerRow` 가 그 id 를 자기 DB 에서 못 찾아 거절하거나, 같은 id 가
   * 우연히 있으면 **엉뚱한 upstream** 으로 이어진다).
   */
  upstreams: { id: string; name: string; resolveAtRuntime?: boolean }[];
  runtimeDns?: RuntimeDnsStatus;
  nodeVersionFamily?: "1.19" | "2.0";
  /** TLS 후보. 같은 이유로 그 서버와 같은 출처여야 한다(`toCertificateOptions`). */
  certificates: CertificateOption[];
  defaultValues?: ServerFormValues;
  /** Where "취소" links back to (the list for /new, the detail page for /edit). */
  cancelHref: string;
  submitLabel?: string;
  nodeName?: string;
  target?: OperationsTarget;
  legacyNode?: boolean;
}

export function ServerForm({
  action,
  runtimeDns,
  nodeVersionFamily,
  upstreams,
  certificates,
  defaultValues: defaults,
  cancelHref,
  submitLabel = "저장",
  nodeName,
  target,
  legacyNode = false,
}: ServerFormProps) {
  const [defaultValues] = useState(defaults);
  // A props refresh must not advance the comparison version or directive carry of the retained draft.
  const [version] = useState(defaultValues?.version);
  const [renderedFamily] = useState(nodeVersionFamily);
  const runAction = useApiAction<NodeActionState>(action);
  const { toast } = useToast();
  const formRef = useRef<HTMLFormElement>(null);
  const [state, formAction, pending] = useActionState(
    async (previous: NodeActionState, form: FormData) => {
      guard.begin(form);
      const result = await runAction(previous, form);
      guard.finish(result.ok);
      if (result.ok && converted)
        toast({
          severity: "success",
          title:
            "저장했습니다 — 이 서버를 경로별 형식으로 전환했습니다. nginx 설정은 같습니다. 「설정 적용」으로 nginx에 반영하세요.",
        });
      return result;
    },
    INITIAL,
  );
  const guard = useProxyFormGuard(formRef, pending);
  const saveErrorRef = useRef<HTMLDivElement>(null);

  const hasLocations =
    !!defaultValues?.locations && defaultValues.locations.length > 0;
  const converted =
    defaultValues?.convertedFromSingle === true ||
    (!!defaultValues?.id && !hasLocations);
  const [listen, setListen] = useState(String(defaultValues?.listen ?? 80));
  const [locations, setLocations] = useState<LocationInput[]>(
    hasLocations
      ? defaultValues!.locations!
      : [
          defaultValues
            ? {
                path: "/",
                mode: defaultValues.upstreamId ? "upstream" : "direct",
                upstreamId: defaultValues.upstreamId ?? "",
                proxyPass: defaultValues.proxyPass ?? "",
                resolveAtRuntime: defaultValues.resolveAtRuntime,
                directives:
                  defaultValues.locationDirectives ?? emptyDirectiveState(),
              }
            : newLocationInput(),
        ],
  );
  const [openRoutes, setOpenRoutes] = useState<ReadonlySet<number>>(
    () => new Set([0]),
  );
  const [serverDir, setServerDir] = useState<DirectiveEditorState>(
    defaultValues?.serverDirectives ?? emptyDirectiveState(),
  );
  // TLS 섹션 state. server_name은 인증서 도메인 커버리지 경고 계산을 위해 제어형.
  const [serverName, setServerName] = useState(defaultValues?.serverName ?? "");
  // Keep absent values absent until edited, preserving legacy advanced gzip.
  const [cache, setCache] = useState<CacheConfig | undefined>(
    defaultValues?.cache,
  );
  const [gzip, setGzip] = useState<GzipConfig | undefined>(defaultValues?.gzip);
  const [certificateId, setCertificateId] = useState(
    defaultValues?.certificateId ?? "",
  );
  const [tlsPort, setTlsPort] = useState(String(defaultValues?.tlsPort ?? 443));
  const [redirectHttp, setRedirectHttp] = useState(
    defaultValues?.redirectHttp !== false,
  );
  // TLS 튜닝(HSTS/ssl_protocols/ssl_ciphers/prefer_server_ciphers) — 인증서 연결 시에만 노출.
  const [hstsEnabled, setHstsEnabled] = useState(
    defaultValues?.tlsOptions?.hsts?.enabled ?? false,
  );
  const [hstsMaxAge, setHstsMaxAge] = useState(
    String(defaultValues?.tlsOptions?.hsts?.maxAge ?? 63072000),
  );
  const [hstsSub, setHstsSub] = useState(
    defaultValues?.tlsOptions?.hsts?.includeSubDomains ?? false,
  );
  const [hstsPreload, setHstsPreload] = useState(
    defaultValues?.tlsOptions?.hsts?.preload ?? false,
  );
  const [sslProtocols, setSslProtocols] = useState(
    defaultValues?.tlsOptions?.sslProtocols ?? "",
  );
  const [sslCiphers, setSslCiphers] = useState(
    defaultValues?.tlsOptions?.sslCiphers ?? "",
  );
  const [preferServer, setPreferServer] = useState(
    defaultValues?.tlsOptions?.preferServerCiphers ?? false,
  );

  // preload는 브라우저 preload 목록 등재 요건(max-age ≥ 1년 + includeSubDomains)을
  // 충족할 때만 유효 — 가드레일: 요건 미충족이면 preload 플래그를 강제로 꺼서 보낸다.
  const HSTS_PRELOAD_MIN_MAX_AGE = 31536000; // packages/nginx/src/tls-presets.ts와 동일 값(1년)
  const preloadAllowed =
    Number(hstsMaxAge) >= HSTS_PRELOAD_MIN_MAX_AGE && hstsSub;
  const tlsOptions: TlsTuning = {
    hsts: hstsEnabled
      ? {
          enabled: true,
          // Blank/non-finite input uses the existing two-year default; zero clears HSTS.
          maxAge:
            hstsMaxAge.trim() === "" || !Number.isFinite(Number(hstsMaxAge))
              ? 63072000
              : Number(hstsMaxAge),
          includeSubDomains: hstsSub,
          preload: hstsPreload && preloadAllowed,
        }
      : undefined,
    sslProtocols:
      sslProtocols === "intermediate" || sslProtocols === "modern"
        ? sslProtocols
        : undefined,
    sslCiphers:
      sslCiphers === "intermediate" || sslCiphers === "modern"
        ? sslCiphers
        : undefined,
    preferServerCiphers: preferServer,
  };

  // IP 접근 제어(서버 레벨) — mode(없음/whitelist/blacklist) + 한 줄에 하나인 IP/CIDR 목록.
  // ipEntries는 텍스트박스가 그대로 편집하는 원본 문자열(줄바꿈 보존); ipAccess는 매 렌더마다
  // trim+빈 줄 제거로 파생되는 제출용 값 — hidden input에만 쓰이고 텍스트박스 값을 되돌리지 않는다.
  const [ipMode, setIpMode] = useState<"" | "whitelist" | "blacklist">(
    defaultValues?.ipAccess?.mode ?? "",
  );
  const [ipEntries, setIpEntries] = useState(
    (defaultValues?.ipAccess?.entries ?? []).join("\n"),
  );
  const ipAccess = ipMode
    ? {
        mode: ipMode,
        entries: ipEntries
          .split(/\r?\n/)
          .map((s) => s.trim())
          .filter(Boolean),
      }
    : undefined;

  // 엣지/프록시 뒤 배치 real IP 복원. 헤더 목록은 packages/nginx/src/real-ip-presets.ts의
  // REAL_IP_HEADERS와 동일하게 유지(런타임 index import는 node-only라 로컬 리터럴 사용 —
  // HSTS_PRELOAD_MIN_MAX_AGE와 동일한 이유).
  const [realIpMode, setRealIpMode] = useState<"" | "cloudflare" | "custom">(
    defaultValues?.realIp?.preset ?? "",
  );
  const [realIpRanges, setRealIpRanges] = useState(
    (defaultValues?.realIp?.trustedRanges ?? []).join("\n"),
  );
  const [realIpHeader, setRealIpHeader] = useState<RealIpHeader>(
    defaultValues?.realIp?.header ?? "X-Forwarded-For",
  );
  const [realIpRecursive, setRealIpRecursive] = useState(
    defaultValues?.realIp?.recursive ?? false,
  );
  const realIp: RealIpConfig | undefined =
    realIpMode === "cloudflare"
      ? { preset: "cloudflare" }
      : realIpMode === "custom"
        ? {
            preset: "custom",
            trustedRanges: realIpRanges
              .split(/\r?\n/)
              .map((s) => s.trim())
              .filter(Boolean),
            header: realIpHeader,
            recursive: realIpRecursive,
          }
        : undefined;
  const selectedCert = certificates.find((c) => c.id === certificateId);
  const uncovered = selectedCert
    ? serverName
        .split(/\s+/)
        .filter(
          (n) =>
            n &&
            !selectedCert.domains.some((domain) =>
              certificateCoversHostname(domain, n),
            ),
        )
    : [];

  const fieldErrors = state.fieldErrors;
  const hasGeneralError =
    !state.ok &&
    Boolean(
      state.nodeError ||
      (state.error && !Object.keys(fieldErrors ?? {}).length),
    );
  function jumpToSaveError() {
    requestAnimationFrame(() => {
      saveErrorRef.current?.scrollIntoView({ block: "start" });
      saveErrorRef.current?.focus({ preventScroll: true });
    });
  }
  function jump(id: string, index?: number) {
    if (index !== undefined)
      setOpenRoutes((previous) => new Set([...previous, index]));
    requestAnimationFrame(() => {
      const element = formRef.current?.querySelector<HTMLElement>(`#${id}`);
      const focus = element?.matches("input,button,textarea,summary,[tabindex]")
        ? element
        : (element?.querySelector<HTMLElement>('[aria-invalid="true"]') ??
          element?.querySelector<HTMLElement>(
            'input:not([type="hidden"]),textarea,button[aria-haspopup="listbox"],summary,button[role="switch"]',
          ));
      for (
        let parent = focus?.parentElement;
        parent;
        parent = parent.parentElement
      ) {
        if (parent instanceof HTMLDetailsElement) parent.open = true;
        if (parent === formRef.current) break;
      }
      (focus ?? element)?.scrollIntoView({ block: "center" });
      focus?.focus({ preventScroll: true });
    });
  }
  const onRoute = (index: number) =>
    jump(`route-editor-heading-${index}`, index);
  const errorItems: SummaryItem[] = Object.entries(fieldErrors ?? {}).map(
    ([key, message]) => {
      const index = locations.findIndex((_, i) =>
        locationError({ [key]: message }, i),
      );
      const id =
        index >= 0
          ? `route-${/\.path$/.test(key) ? "path" : locations[index]!.mode === "upstream" ? "upstream" : "proxy"}-${index}`
          : key === "serverName"
            ? "server-name"
            : key === "listen"
              ? "server-listen"
              : key === "certificateId"
                ? "server-certificate"
                : key === "tlsPort"
                  ? "server-tls-port"
                  : key.startsWith("tlsOptions")
                    ? "server-certificate"
                    : key.startsWith("ipAccess")
                      ? ipMode
                        ? "ip-entries"
                        : "ip-mode"
                      : key.startsWith("realIp")
                        ? realIpMode === "custom"
                          ? "realip-ranges"
                          : "realip-mode"
                        : key.startsWith("cache")
                          ? "cache-size"
                          : key.startsWith("gzip")
                            ? "gzip-min-length"
                            : key.startsWith("locations")
                              ? `route-${locations[0]?.mode === "upstream" ? "upstream" : "proxy"}-0`
                              : "server-directives-advanced";
      return {
        label:
          index >= 0
            ? `경로 ${index + 1} · ${locations[index]!.path}`
            : ({
                serverName: "server_name",
                listen: "listen (포트)",
                certificateId: "인증서",
                tlsPort: "HTTPS 포트",
                redirectHttp: "HTTP→HTTPS 리다이렉트",
                tlsOptions: "TLS 세부 설정",
                ipAccess: "IP 접근 제어",
                realIp: "엣지 / 프록시 뒤 배치 (실 IP 복원)",
                cache: "프록시 캐시",
                gzip: "gzip",
                serverDirectives: "고급 — 추가 디렉티브 직접 입력",
                locations: "위치",
              }[key.split(".")[0]!] ?? "서버"),
        value: message,
        onJump: () =>
          jump(
            id,
            index >= 0 ? index : key.startsWith("locations") ? 0 : undefined,
          ),
      };
    },
  );
  locations.forEach((route, index) => {
    if (
      route.mode === "upstream" &&
      (route.upstreamError ||
        (route.upstreamId && !upstreams.some((u) => u.id === route.upstreamId)))
    )
      errorItems.push({
        label: `경로 ${index + 1} · ${route.path}`,
        value:
          "연결된 Upstream을 찾을 수 없습니다(삭제됐거나 이 서버 범위에 없음). 다른 Upstream을 고르거나 직접 주소로 바꾸세요.",
        onJump: () => onRoute(index),
      });
  });
  useEffect(() => {
    if (state.fieldErrors && Object.keys(state.fieldErrors).length)
      errorItems[0]?.onJump();
    else if (state.error || state.nodeError) jumpToSaveError();
  }, [state]);
  const model = serverSummary({
    serverName,
    listen,
    locations,
    upstreams,
    serverDirectives: serverDir,
    originalServerDirectives: defaultValues?.originalServerDirectives,
    cache,
    gzip,
  });
  const summary: SummaryItem[] = [
    ...[
      {
        label: "저장 위치",
        value: nodeName ?? "이 서버 (local)",
        onJump: () => jump("server-form-identity"),
      },
    ],
    {
      label: "주소",
      value: model.names.length
        ? `${certificateId ? "https" : "http"}://${model.names[0]}${certificateId ? (tlsPort !== "443" ? `:${tlsPort}` : "") : listen !== "80" ? `:${listen}` : ""}${model.names.length > 1 ? ` 외 ${model.names.length - 1}개` : ""}`
        : "도메인 미입력",
      onJump: () => jump("server-name"),
    },
    {
      label: "수신",
      value: certificateId
        ? `HTTPS :${tlsPort} · HTTP :${listen}${redirectHttp ? " 리다이렉트" : " 사용 안 함"}`
        : `HTTP :${listen}`,
      onJump: () => jump("server-form-https"),
    },
    ...locations.map((route, index) => ({
      label: `경로 ${index + 1}`,
      value: (
        <>
          {route.path || "(경로 입력)"} → {model.locations[index]!.target}
          {model.locations[index]!.retryOtherInstances === true ? (
            <small>{model.locations[index]!.retryInfo.summary}</small>
          ) : null}
          {model.locations[index]!.directives.webSocket !== "꺼짐" ? (
            <small>
              WebSocket {model.locations[index]!.directives.webSocket}
            </small>
          ) : null}
          {route.cacheEnabled !== undefined ? (
            <small>캐시 {route.cacheEnabled ? "켜기" : "끄기"}</small>
          ) : null}
          {route.gzipEnabled !== undefined ? (
            <small>gzip {route.gzipEnabled ? "켜기" : "끄기"}</small>
          ) : null}
          {route.ipAccess ? (
            <small>IP {route.ipAccess.entries.length}개</small>
          ) : null}
        </>
      ),
      onJump: () => onRoute(index),
    })),
    {
      label: "HTTPS",
      value: certificateId ? (
        <>
          {selectedCert?.label} · :{tlsPort}
          {hstsEnabled ? " · HSTS" : ""}
          {uncovered.length ? (
            <small>인증서 밖 이름 {uncovered.length}개</small>
          ) : null}
        </>
      ) : (
        "사용 안 함"
      ),
      onJump: () => jump("server-form-https"),
    },
    {
      label: "앞단",
      value:
        realIpMode === "cloudflare"
          ? "Cloudflare"
          : realIpMode === "custom"
            ? `직접 입력 · 신뢰 대역 ${realIp?.trustedRanges?.length ?? 0}개`
            : "없음 (직접 노출)",
      onJump: () => jump("server-form-access"),
    },
    {
      label: "IP 접근",
      value: ipMode
        ? `${ipMode === "whitelist" ? "허용" : "차단"} 목록 ${ipAccess?.entries.length}개`
        : "제한 없음",
      onJump: () => jump("server-form-access"),
    },
    {
      label: "응답 캐시",
      value: `${model.cache} · 경로 예외 ${model.cacheOverrides}개`,
      onJump: () => jump("server-form-cache"),
    },
    {
      label: "server",
      value:
        [
          model.serverDirectives.bodySize
            ? `본문 ${model.serverDirectives.bodySize}`
            : "",
          model.serverDirectives.timeouts.read
            ? `읽기 ${model.serverDirectives.timeouts.read}`
            : "",
          model.serverDirectives.responseHeaderNames.length
            ? `응답 헤더 ${model.serverDirectives.responseHeaderNames.join(", ")}`
            : "",
          model.serverDirectives.advancedLineCount
            ? `고급 ${model.serverDirectives.advancedLineCount}줄`
            : "",
        ]
          .filter(Boolean)
          .join(" · ") || "추가 디렉티브 없음",
      onJump: () => jump("server-form-common"),
    },
    {
      label: "gzip",
      value: `${model.gzip} · 경로 예외 ${model.gzipOverrides}개`,
      onJump: () => jump("server-form-gzip"),
    },
  ];
  function updateLocation(index: number, patch: Partial<LocationInput>) {
    setLocations((rows) =>
      rows.map((row, i) =>
        i === index
          ? {
              ...row,
              ...patch,
              ...(patch.mode || patch.upstreamId
                ? { upstreamError: undefined }
                : {}),
            }
          : row,
      ),
    );
  }
  const activeRuntimeDns = locations.some((route) =>
    route.mode === "direct"
      ? route.resolveAtRuntime === true
      : upstreams.find((u) => u.id === route.upstreamId)?.resolveAtRuntime ===
        true,
  );
  return (
    <form
      ref={formRef}
      action={formAction}
      className={`${layout.formWithRail} ${formStyles.pfForm}`}
      aria-busy={pending}
      noValidate
    >
      {renderedFamily && (
        <input type="hidden" name="nodeVersionFamily" value={renderedFamily} />
      )}
      {defaultValues?.id ? (
        <input type="hidden" name="id" value={defaultValues.id} />
      ) : null}
      {version !== undefined ? (
        <input type="hidden" name="version" value={version} />
      ) : null}
      <input type="hidden" name="targetMode" value="locations" />
      <input
        type="hidden"
        name="serverDirectives"
        value={JSON.stringify(
          carryDirectives(defaultValues?.originalServerDirectives, serverDir),
        )}
      />
      <input
        type="hidden"
        name="locations"
        value={serializeLocations(locations)}
      />
      <input
        type="hidden"
        name="ipAccess"
        value={ipAccess ? JSON.stringify(ipAccess) : ""}
      />
      <input
        type="hidden"
        name="realIp"
        value={realIp ? JSON.stringify(realIp) : ""}
      />
      <input
        type="hidden"
        name="cache"
        value={cache ? JSON.stringify(cache) : ""}
      />
      <input
        type="hidden"
        name="gzip"
        value={gzip ? JSON.stringify(gzip) : ""}
      />
      <input type="hidden" name="redirectHttp" value={String(redirectHttp)} />
      {certificateId ? (
        <input
          type="hidden"
          name="tlsOptions"
          value={JSON.stringify(tlsOptions)}
        />
      ) : null}
      {hasGeneralError || runtimeDns?.available !== true ? (
        <div className={formStyles.pfWide}>
          {hasGeneralError ? (
            <div
              ref={saveErrorRef}
              className={formStyles.pfSaveMessage}
              role="group"
              aria-label="저장 오류"
              tabIndex={-1}
            >
              {state.nodeError ? (
                <NodeErrorCard {...state.nodeError} />
              ) : (
                <Banner tone="error" role="alert" title={state.error} />
              )}
            </div>
          ) : null}
          <RuntimeDnsNotice status={runtimeDns} active={activeRuntimeDns} />
        </div>
      ) : null}
      <fieldset
        className={formStyles.pfMain}
        disabled={pending}
        inert={pending}
      >
        <FormStack>
          <Banner tone="info" title={UNAPPLIED_CONFIG_INFO} />
          <IdentityStrip
            nodeName={nodeName}
            serverName={serverName}
            onServerNameChange={setServerName}
            listen={listen}
            onListenChange={setListen}
            https={!!certificateId}
            tlsPort={tlsPort}
            certificate={selectedCert?.label}
            redirectHttp={redirectHttp}
            onTls={() => jump("server-form-https")}
            uncovered={uncovered.length}
            fieldErrors={fieldErrors}
          />
          <ProtectView
            certificateBaseHref={`${cancelHref.split("/reverse-proxy")[0]}/certificates`}
            certificates={certificates}
            tls={{
              certificateId,
              setCertificateId,
              tlsPort,
              setTlsPort,
              redirectHttp,
              setRedirectHttp,
              sslProtocols,
              setSslProtocols,
              sslCiphers,
              setSslCiphers,
              preferServer,
              setPreferServer,
              hstsEnabled,
              setHstsEnabled,
              hstsMaxAge,
              setHstsMaxAge,
              hstsSub,
              setHstsSub,
              hstsPreload,
              setHstsPreload,
            }}
            access={{
              ipMode,
              setIpMode,
              ipEntries,
              setIpEntries,
              realIpMode,
              setRealIpMode,
              realIpRanges,
              setRealIpRanges,
              realIpHeader,
              setRealIpHeader,
              realIpRecursive,
              setRealIpRecursive,
            }}
            preloadAllowed={preloadAllowed}
            uncoveredNames={uncovered}
            fieldErrors={fieldErrors}
            locations={locations}
            onRoute={onRoute}
          />
          <div id="server-form-routes" tabIndex={-1}>
            <RoutesView
              locations={locations}
              upstreams={upstreams}
              open={openRoutes}
              onToggle={(index) =>
                setOpenRoutes((previous) => {
                  const next = new Set(previous);
                  if (next.has(index)) next.delete(index);
                  else next.add(index);
                  return next;
                })
              }
              onAdd={() => {
                setLocations((rows) => [...rows, newLocationInput()]);
                setOpenRoutes(
                  (previous) => new Set([...previous, locations.length]),
                );
              }}
              onRemove={(index) => {
                setLocations((rows) => rows.filter((_, i) => i !== index));
                setOpenRoutes(
                  (previous) =>
                    new Set(
                      [...previous]
                        .filter((i) => i !== index)
                        .map((i) => (i > index ? i - 1 : i)),
                    ),
                );
              }}
              onMove={(index, offset) => {
                const next = index + offset;
                if (next < 0 || next >= locations.length) return;
                setLocations((rows) => {
                  const moved = [...rows];
                  [moved[index], moved[next]] = [moved[next]!, moved[index]!];
                  return moved;
                });
                setOpenRoutes(
                  (previous) =>
                    new Set(
                      [...previous].map((i) =>
                        i === index ? next : i === next ? index : i,
                      ),
                    ),
                );
              }}
              converted={converted}
              fieldErrors={fieldErrors}
              editor={(index) => (
                <RouteEditor
                  location={locations[index]!}
                  index={index}
                  nodeName={nodeName}
                  absent={
                    locations[index]!.originalDirectives === null &&
                    directiveStateUnchanged(null, locations[index]!.directives)
                  }
                  carried={
                    Array.isArray(locations[index]!.originalDirectives) &&
                    directiveStateUnchanged(
                      locations[index]!.originalDirectives,
                      locations[index]!.directives,
                    )
                  }
                  upstreams={upstreams}
                  runtimeDns={runtimeDns}
                  legacyNode={legacyNode}
                  serverDirectives={serverDir}
                  cacheEnabled={cache?.enabled === true}
                  gzipEnabled={gzip?.enabled === true}
                  error={locationError(fieldErrors, index)}
                  onChange={(patch) => updateLocation(index, patch)}
                />
              )}
            />
          </div>
          <CommonView
            cache={cache}
            gzip={gzip}
            onCacheChange={setCache}
            onGzipChange={setGzip}
            legacyGzip={
              model.gzip === "직접 설정" ||
              /(?:^|[;\n])\s*gzip(?:_[a-z_]+)?\s/m.test(serverDir.advanced)
            }
            serverDirectives={serverDir}
            onServerDirectivesChange={setServerDir}
            locations={locations}
            onRoute={onRoute}
            fieldErrors={fieldErrors}
          />
        </FormStack>
      </fieldset>
      <SummaryRail
        target={target}
        items={summary}
        errors={errorItems}
        onSaveErrorJump={hasGeneralError ? jumpToSaveError : undefined}
        converted={converted}
        pending={pending}
        cancelHref={cancelHref}
        submitLabel={submitLabel}
      />
    </form>
  );
}
