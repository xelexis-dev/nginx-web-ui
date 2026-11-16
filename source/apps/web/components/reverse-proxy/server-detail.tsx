import layout from "@/components/page-layout.module.css";
import Link from "next/link";
import type {
  RuntimeDnsStatus,
  NodeLocation,
  ServerDetailView,
} from "@repo/contracts";
import { RuntimeDnsNotice } from "./runtime-dns";
import { runtimeDnsModeLabel } from "./runtime-dns-state";
import {
  linkClassName,
  Banner,
  CodePane,
  Facts,
  Icon,
  PageHeader,
  StatusBadge,
  Surface,
  Table,
  Td,
  Th,
  Tr,
} from "@repo/ui";
import { configPreview, type ConfigPreview } from "@/lib/config-preview";
import {
  parseCacheConfig,
  parseGzipConfig,
} from "@repo/presentation/nginx/performance";
import type { NodeFormAction } from "@/lib/node-action-state";
import { ServerDetailActions } from "./detail-actions";
import { locationTarget, type StoredServerConfig } from "./server-vm";
import styles from "@/app/(app)/reverse-proxy/reverse-proxy.module.css";

/**
 * Shared local/node detail (Operations): header with status and write actions, the saved settings
 * beside the generated nginx config (pinned at >=1024px), per-location table. nginx previews are
 * rendered by the target API.
 */

/** 참조 가능한 upstream 하나. 이름 조회(대상 라벨)와 블록 렌더에 둘 다 쓰인다. */
export interface ServerDetailUpstreamVM {
  resolveAtRuntime?: boolean;
  id: string;
  name: string;
  servers: string[];
}

/**
 * 연결된 인증서. **`certificateId` 가 있는데 이 값이 `null` 이면 「삭제됨」**이다 —
 * 라벨이 그 상태를 따로 말한다(로컬 추출 전과 같은 분기).
 */
export interface ServerDetailCertVM {
  id: string;
  domains: string[];
  staging: boolean;
  /** `status === "issued"`. 라벨의 「연결됨 …」 / 「발급 대기·실패」 분기. */
  issued: boolean;
  /** Target API certificate readiness, without PEM material. */
  tlsReady: boolean;
}

export interface ServerDetailVM extends Pick<
  ServerDetailView,
  "selfProxyUncheckedPaths"
> {
  preview?: ConfigPreview;
  id: string;
  serverName: string;
  listen: number;
  enabled: boolean;
  /** `nginx_servers.config` jsonb — 이미 좁혀진 것이 온다(`storedConfig`). */
  config: StoredServerConfig;
  upstreamId: string | null;
  certificateId: string | null;
  /** 이 게이트웨이/노드의 upstream **전부**. `http://<name>` 로 참조된 것까지 블록에 싣는다. */
  upstreams: ServerDetailUpstreamVM[];
  cert: ServerDetailCertVM | null;
  /**
   * 「이 서버가 관리 UI 자기 자신을 가리킨다」 안내를 띄울지.
   *
   * **로컬 화면만 `true` 를 줄 수 있다.** 판정에 그 인스턴스의 「집계 제외 도메인」
   * 설정이 필요한데(이미 제외해 둔 운영자에게 계속 잔소리하지 않으려고), 그 값은
   * `/api/v1` 어디에도 없다 — 노출하면 계약 변경이다. 게다가 안내 문구가 가리키는
   * 「사이트 설정 → 일반」은 **게이트웨이 자신의** 설정 화면이라, 노드 화면에서
   * 띄우면 엉뚱한 서버의 설정을 고치라고 말하게 된다. 그래서 노드는 항상 `false`다.
   */
  selfProxyNotice: boolean;
}

/**
 * 쓰기 UI 한 덩어리(수정 링크 + 토글/삭제). **쪼개지 않은 이유는
 * `ServerListWrite` 와 같다** — 「버튼은 그렸는데 갈 곳이 없다」를 타입이 못 잡는
 * 조합을 아예 못 만들게 한다.
 */
export interface ServerDetailWrite {
  /**
   * 「수정」이 갈 곳.
   *
   * **`basePath` 에서 파생하지 않고 필드로 들고 있는 것이 요점이다.** 파생하면
   * 그 경로가 아직 없는 화면도 `write` 만 채우면 버튼이 그려지고, 아무 검사도
   * 그걸 못 잡는다 — 이 묶음이 막으려는 바로 그 조합이다. 목적지를 값으로
   * 적게 하면 「없는 곳을 가리키는 버튼」을 만들려면 **그 주소를 손으로 써야**
   * 한다(`ServerListWrite.templatesHref` 가 원래 그렇게 생겼다).
   */
  editHref: string;
  /** 로컬은 `toggleServer`, 노드는 `toggleNodeServer.bind(null, nodeId)`. */
  toggleAction: NodeFormAction;
  /** 로컬은 `deleteServer`, 노드는 `deleteNodeServer.bind(null, nodeId)`. */
  deleteAction: NodeFormAction;
}

export type ServerConfigOrigin = "this-instance" | "remote-node";

export interface ServerDetailProps {
  runtimeDns?: RuntimeDnsStatus;
  data: ServerDetailVM;
  /** `/reverse-proxy`(로컬) 또는 `/nodes/<id>/reverse-proxy`(노드). */
  basePath: string;
  /** `null` 이면 수정·토글·삭제를 통째로 그리지 않는다(`ServerDetailWrite` 문서). */
  write: ServerDetailWrite | null;
  /** `ServerConfigOrigin` 문서. **기본값을 두지 않는다** — 화면이 자기가 어느 쪽인지 말해야 한다. */
  configOrigin: ServerConfigOrigin;
}

export function ServerDetail({
  data,
  runtimeDns,
  basePath,
  write,
  configOrigin,
}: ServerDetailProps) {
  const cfg = data.config;
  const upstreamById = new Map(data.upstreams.map((u) => [u.id, u]));
  const upstream = data.upstreamId
    ? (upstreamById.get(data.upstreamId) ?? null)
    : null;

  const hasLocations = Array.isArray(cfg.locations) && cfg.locations.length > 0;
  const targetLabel = hasLocations
    ? `location별 (${cfg.locations!.length}개)`
    : data.upstreamId
      ? `upstream: ${upstream?.name ?? data.upstreamId}`
      : cfg.proxyPass
        ? `proxy_pass: ${cfg.proxyPass}`
        : "—";

  const upstreamByUrl = new Map(
    data.upstreams.map((group) => [`http://${group.name}`, group]),
  );
  const upstreamNames = new Set(data.upstreams.map((group) => group.name));
  const locationGroup = (location: NodeLocation) =>
    location.upstreamId !== undefined
      ? location.upstreamName
        ? upstreamById.get(location.upstreamId)
        : undefined
      : upstreamByUrl.get(location.proxyPass ?? "");
  const locationDnsMode = (location: NodeLocation) => {
    if (locationTarget(location, upstreamNames).missingUpstream)
      return "Upstream 없음";
    const group = locationGroup(location);
    return group || location.upstreamId !== undefined
      ? `Upstream 설정 따름 · ${group ? runtimeDnsModeLabel(group.resolveAtRuntime) : "설정 확인 필요"}`
      : runtimeDnsModeLabel(location.resolveAtRuntime);
  };
  const runtimeMode = hasLocations
    ? "경로별 설정"
    : data.upstreamId
      ? `Upstream 설정 따름 · ${upstream ? runtimeDnsModeLabel(upstream.resolveAtRuntime) : "설정 확인 필요"}`
      : runtimeDnsModeLabel(cfg.resolveAtRuntime);
  const activeRuntimeDns = hasLocations
    ? cfg.locations!.some((location) => {
        if (location.upstreamId !== undefined)
          return (
            !locationTarget(location, upstreamNames).missingUpstream &&
            locationGroup(location)?.resolveAtRuntime === true
          );
        const group = locationGroup(location);
        return group
          ? group.resolveAtRuntime === true
          : location.resolveAtRuntime === true;
      })
    : data.upstreamId
      ? upstream?.resolveAtRuntime === true
      : cfg.resolveAtRuntime === true;

  const serverDirCount = Array.isArray(cfg.serverDirectives)
    ? cfg.serverDirectives.length
    : 0;
  const locationDirCount = hasLocations
    ? cfg.locations!.reduce(
        (n, l) => n + (Array.isArray(l.directives) ? l.directives.length : 0),
        0,
      )
    : Array.isArray(cfg.locationDirectives)
      ? cfg.locationDirectives.length
      : 0;
  const directiveLabel = `server ${serverDirCount}개 · location ${locationDirCount}개`;

  const cert = data.cert;
  const cfgTls = cfg as { tlsPort?: number };
  const tlsPort = typeof cfgTls.tlsPort === "number" ? cfgTls.tlsPort : 443;
  const tlsLabel = !data.certificateId
    ? "없음 (HTTP)"
    : !cert
      ? "연결된 인증서가 삭제됨 — HTTP로 동작 중"
      : cert.issued
        ? `연결됨: ${cert.domains[0] ?? cert.id} (포트 ${tlsPort}${cert.staging ? " · STAGING" : ""})`
        : `연결됨: ${cert.domains[0] ?? cert.id} (발급 대기/실패 — HTTP로 동작 중)`;
  // 인증서를 가리키는데 HTTPS 로 동작하지 않는 두 경우는 머리말 아래에서도 말한다.
  const tlsProblem =
    data.certificateId && (!cert || !cert.issued) ? tlsLabel : null;

  const realIpCfg = cfg.realIp;
  const realIpLabel = !realIpCfg
    ? "없음 (직접 노출)"
    : realIpCfg.preset === "cloudflare"
      ? "Cloudflare (내장 대역 · CF-Connecting-IP)"
      : `커스텀 (${realIpCfg.trustedRanges?.length ?? 0}개 대역 · ${realIpCfg.header ?? "X-Forwarded-For"})`;

  let cacheLabel = "설정 확인 필요";
  let gzipLabel = "설정 확인 필요";
  try {
    const cache = parseCacheConfig(cfg.cache);
    cacheLabel = cache?.enabled
      ? `켜짐 · ${cache.maxSize} · 유효기간 ${cache.valid}`
      : "꺼짐";
  } catch {
    /* Unsupported node settings remain visible through the preview error below. */
  }
  try {
    const gzip = parseGzipConfig(cfg.gzip);
    gzipLabel = gzip
      ? gzip.enabled
        ? `켜짐 · 압축 수준 ${gzip.compLevel}`
        : "꺼짐"
      : cfg.serverDirectives?.some((d) => d.name === "gzip")
        ? "고급 디렉티브 설정 사용"
        : "꺼짐";
  } catch {
    /* Do not display unvalidated jsonb as an enabled/disabled setting. */
  }

  const {
    config: renderedConfig,
    error: previewError,
    unresolvedPaths,
  } = configPreview(data.preview);
  const names = data.serverName.split(/\s+/).filter(Boolean);

  return (
    <div className={layout.page}>
      <PageHeader
        eyebrow={
          <Link href={basePath} className={linkClassName({ variant: "back" })}>
            <Icon name="back" size={14} />
            리버스 프록시
          </Link>
        }
        title={names[0] ?? data.serverName}
        status={<StatusBadge tone={data.enabled ? (tlsProblem ? "warning" : "success") : "neutral"}>{data.enabled ? "활성" : "비활성"}</StatusBadge>}
        description={names.length > 1 ? names.slice(1).join(" ") : undefined}
        actions={
          write ? (
            <ServerDetailActions
              id={data.id}
              serverName={names[0] ?? data.serverName}
              enabled={data.enabled}
              toggleAction={write.toggleAction}
              deleteAction={write.deleteAction}
              editHref={write.editHref}
            />
          ) : undefined
        }
      />

      {tlsProblem ? (
        <Banner
          tone={cert ? "warning" : "error"}
          title={
            cert
              ? "인증서가 아직 발급되지 않았습니다"
              : "연결된 인증서가 삭제됐습니다"
          }
          role="status"
        >
          {tlsProblem}
        </Banner>
      ) : null}

      {data.selfProxyUncheckedPaths?.length ? (
        <Banner
          tone="info"
          title="일부 경로는 아직 확인하지 않았습니다"
          role="status"
        >
          <p>관리 화면을 가리키는지 확인하지 않은 경로입니다.</p>
          <ul className={styles.opsTargets}>
            {data.selfProxyUncheckedPaths.map((path, index) => (
              <li key={index} className={styles.opsTarget}>
                <code>{path}</code>
              </li>
            ))}
          </ul>
        </Banner>
      ) : null}

      {data.selfProxyNotice ? (
        <Banner
          tone="info"
          title="이 서버는 이 관리 화면 자신을 가리킵니다"
          role="status"
        >
          그러면 요청이 nginx 를 두 번 지나{" "}
          <strong>접속 지표에서 이 도메인의 요청 수가 약 두 배로</strong>{" "}
          잡힙니다.{" "}
          <Link href="/admin/settings/general" className={linkClassName()}>
            사이트 설정 → 일반
          </Link>
          의 「집계 제외 도메인」에 <code>{data.serverName}</code> 을 넣으면
          정확해집니다. 로그·차단·설정에는 영향이 없고, 지표 집계에만
          해당합니다.
        </Banner>
      ) : null}

      <RuntimeDnsNotice status={runtimeDns} active={activeRuntimeDns} />

      <div className={styles.opsDetailMain}>
          <Surface
            title="설정 요약"
            info="저장된 설정입니다. 트래픽은 접속 지표에서 봅니다."
          >
            <Facts
            columns={2}
              items={[
                { term: "server_name", value: data.serverName, mono: true },
                {
                  term: "listen",
                  value: data.cert?.issued
                    ? `${data.listen} → ${tlsPort} (HTTPS)`
                    : String(data.listen),
                },
                { term: "대상", value: targetLabel, mono: !hasLocations },
                { term: "이름 확인", value: runtimeMode },
                { term: "커스텀 디렉티브", value: directiveLabel },
                { term: "TLS", value: tlsLabel },
                { term: "엣지/실 IP", value: realIpLabel },
                { term: "서버 기본 캐시", value: cacheLabel },
                { term: "서버 기본 gzip", value: gzipLabel },
                { term: "상태", value: data.enabled ? "활성" : "비활성" },
              ]}
            />
          </Surface>

          {hasLocations ? (
            <Surface title="경로" info="경로별 대상과 캐시 / 압축" flush>
              <Table label="경로별 대상과 캐시 / 압축">
                <thead>
                  <tr>
                    <Th>경로</Th>
                    <Th>대상</Th>
                    <Th>이름 확인</Th>
                    <Th>캐시</Th>
                    <Th>gzip</Th>
                  </tr>
                </thead>
                <tbody>
                  {cfg.locations!.map((loc, index) => {
                    const target = locationTarget(loc, upstreamNames);
                    return (
                      <Tr key={`${index}-${loc.path}`}>
                        <Td cell="primary" mono>
                          {loc.path}
                        </Td>
                        <Td mono truncate label="대상" title={target.label}>
                          {target.missingUpstream ? (
                            <StatusBadge
                              tone="warning"
                              variant="outline"
                              size="sm"
                              dot={false}
                            >
                              Upstream 없음
                            </StatusBadge>
                          ) : (
                            target.label
                          )}
                        </Td>
                        <Td label="이름 확인">{locationDnsMode(loc)}</Td>
                        <Td label="캐시">{locationPerformance(loc.cacheEnabled)}</Td>
                        <Td label="gzip">{locationPerformance(loc.gzipEnabled)}</Td>
                      </Tr>
                    );
                  })}
                </tbody>
              </Table>
            </Surface>
          ) : null}

        <CodePane
          title="생성되는 nginx 설정"
          info={configOrigin === "this-instance"
            ? "이 항목의 현재 값으로 렌더된 설정입니다 — 설정 적용 시 nginx에 그대로 기록됩니다."
            : "노드의 현재 값과 인증서 경로로 렌더된 미리보기입니다."}
          description={
            <>
              {!data.enabled
                ? " 이 서버는 비활성 상태라 적용 시 기록되지 않습니다."
                : ""}
              {unresolvedPaths?.length ? (
                <div role="status" className={styles.previewHint}>
                  이름 확인 없이 만든 미리보기입니다 — 실제 적용에서는 이 경로의
                  결과가 달라질 수 있습니다
                  <ul className={styles.opsTargets}>
                    {unresolvedPaths.map((path, index) => (
                      <li className={styles.opsTarget} key={index}>
                        <code>{path}</code>
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
            </>
          }
          code={renderedConfig ?? undefined}
          error={
            previewError
              ? `미리보기를 만들 수 없습니다: ${previewError}`
              : undefined
          }
        />
      </div>
    </div>
  );
}

/** 경로별 캐시·gzip 값의 말. `null`/`undefined` 는 서버 설정을 따른다는 뜻이다. */
function locationPerformance(value: boolean | null | undefined): string {
  return value == null
    ? "서버 설정 따름"
    : value === true
      ? "켜기"
      : value === false
        ? "끄기"
        : "설정 확인 필요";
}
