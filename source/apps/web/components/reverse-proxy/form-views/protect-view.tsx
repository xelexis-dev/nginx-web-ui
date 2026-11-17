"use client";
import { useId } from "react";
import Link from "next/link";
import {
  TextArea,
  TextInput,
  Field,
  Card,
  buttonClassName,
  FieldGroup,
  Banner,
  InfoTip,
  Select,
  SegmentedControl,
  Button,
} from "@repo/ui";
import type { RealIpHeader } from "@repo/presentation/nginx";
import type { CertificateOption, LocationInput } from "../server-form-vm";
import { FormDisclosure } from "../directive-editor";
import layout from "@/components/page-layout.module.css";
import formStyles from "@/components/reverse-proxy/server-form.module.css";
/**
 * Every piece of TLS state the section edits. Bundled into one object because
 * the form container owns all of it (the hidden `tlsOptions` JSON is assembled
 * there, and `certificateId` is also read by the "접근 제어" tab), and eleven
 * separate value/setter prop pairs would be worse to read than one bag.
 */
export interface TlsSectionState {
  certificateId: string;
  setCertificateId: (value: string) => void;
  tlsPort: string;
  setTlsPort: (value: string) => void;
  http2: boolean;
  setHttp2: (value: boolean) => void;
  http2Supported: boolean;
  redirectHttp: boolean;
  setRedirectHttp: (value: boolean) => void;
  sslProtocols: string;
  setSslProtocols: (value: string) => void;
  sslCiphers: string;
  setSslCiphers: (value: string) => void;
  preferServer: boolean;
  setPreferServer: (value: boolean) => void;
  hstsEnabled: boolean;
  setHstsEnabled: (value: boolean) => void;
  hstsMaxAge: string;
  setHstsMaxAge: (value: string) => void;
  hstsSub: boolean;
  setHstsSub: (value: boolean) => void;
  hstsPreload: boolean;
  setHstsPreload: (value: boolean) => void;
}

/**
 * "TLS" tab: certificate link + HTTPS behaviour + cipher/HSTS tuning.
 *
 * Everything below the certificate picker is gated on a certificate actually
 * being selected — without one the server is plain HTTP and none of these
 * settings would be rendered into the config.
 */
function TlsFields({
  certificates,
  tls,
  preloadAllowed,
  preloadMinMaxAge,
  uncoveredNames,
  fieldErrors,
  certificateBaseHref,
}: {
  certificateBaseHref: string;
  certificates: CertificateOption[];
  tls: TlsSectionState;
  /** HSTS preload 요건(max-age ≥ 1년 + includeSubDomains) 충족 여부. */
  preloadAllowed: boolean;
  preloadMinMaxAge: number;
  /** server_name 중 선택한 인증서가 커버하지 않는 이름들. */
  uncoveredNames: string[];
  fieldErrors?: Record<string, string>;
}) {
  const group = useId();
  return (
    <div className={formStyles.directiveGroup}>
      <div className={formStyles.pfFields}>
        <Field
          label="인증서"
          htmlFor="server-certificate"
          error={fieldErrors?.certificateId}
        >
          <Select
            name="certificateId"
            value={tls.certificateId}
            onChange={tls.setCertificateId}
            options={[
              { value: "", label: "(없음 — HTTP)" },
              ...certificates.map((c) => ({
                value: c.id,
                label: c.staging ? `${c.label} [STAGING]` : c.label,
              })),
            ]}
            aria-label="인증서"
          />
        </Field>
        {tls.certificateId ? (
          <Field
            label="HTTPS 포트"
            htmlFor="server-tls-port"
            error={fieldErrors?.tlsPort}
          >
            <TextInput
              name="tlsPort"
              type="number"
              min={1}
              max={65535}
              value={tls.tlsPort}
              onChange={(e) => tls.setTlsPort(e.target.value)}
            />
          </Field>
        ) : null}
      </div>
      <FieldGroup id="server-http2" legend="HTTP/2">
        <SegmentedControl
          label="HTTP/2"
          value={tls.certificateId && tls.http2 ? "on" : "off"}
          onChange={(value) => tls.setHttp2(value === "on")}
          disabled={!tls.certificateId || !tls.http2Supported}
          options={[{ value: "on", label: "사용" }, { value: "off", label: "사용 안 함" }]}
        />
        <p className={formStyles.directiveGroupNote}>
          {!tls.http2Supported
            ? "이 노드는 HTTP/2 설정을 지원하지 않습니다. 노드를 2.1 이상으로 업데이트하세요."
            : !tls.certificateId ? "HTTP/2를 사용하려면 인증서를 선택하세요."
            : "HTTPS 연결에서 HTTP/2를 허용합니다. 저장한 뒤 설정을 적용하세요."}
        </p>
        {fieldErrors?.http2 ? <Banner tone="error" role="alert" title={fieldErrors.http2} /> : null}
      </FieldGroup>
      {tls.certificateId ? (
        <>
          <FieldGroup id="tls-redirect" legend="HTTP→HTTPS 리다이렉트">
            <SegmentedControl
              label="HTTP→HTTPS 리다이렉트"
              value={tls.redirectHttp ? "on" : "off"}
              onChange={(v) => tls.setRedirectHttp(v === "on")}
              options={[
                { value: "on", label: "리다이렉트 (기존 listen 포트)" },
                { value: "off", label: "HTTPS만" },
              ]}
            />
          </FieldGroup>
          <FormDisclosure
            group={group}
            title="TLS 세부 설정"
            summary={`${tls.sslProtocols || "nginx 기본값"} · ciphers ${tls.sslCiphers || "기본값"} · 서버 암호군 ${tls.preferServer ? "우선" : "기본값"}`}
          >
            {" "}
            <Field label="ssl_protocols" htmlFor="ssl-protocols">
              <Select
                value={tls.sslProtocols}
                onChange={tls.setSslProtocols}
                options={[
                  { value: "", label: "기본 (nginx 기본값)" },
                  {
                    value: "intermediate",
                    label: "Intermediate (TLS 1.2+1.3, 권장)",
                  },
                  { value: "modern", label: "Modern (TLS 1.3만)" },
                ]}
                aria-label="ssl_protocols"
              />
            </Field>
            <Field label="ssl_ciphers" htmlFor="ssl-ciphers">
              <Select
                value={tls.sslCiphers}
                onChange={tls.setSslCiphers}
                options={[
                  { value: "", label: "기본" },
                  { value: "intermediate", label: "Mozilla Intermediate" },
                  {
                    value: "modern",
                    label: "Mozilla Modern (TLS1.3 고정, 미출력)",
                  },
                ]}
                aria-label="ssl_ciphers"
              />
            </Field>
            <FieldGroup
              id="tls-prefer-server"
              legend="ssl_prefer_server_ciphers"
            >
              <SegmentedControl
                label="prefer server ciphers"
                value={tls.preferServer ? "on" : "off"}
                onChange={(v) => tls.setPreferServer(v === "on")}
                options={[
                  { value: "on", label: "on" },
                  { value: "off", label: "off (권장)" },
                ]}
              />
            </FieldGroup>
          </FormDisclosure>
          <FormDisclosure
            group={group}
            title="HSTS"
            summary={
              tls.hstsEnabled
                ? `${tls.hstsMaxAge}초 · includeSubDomains ${tls.hstsSub ? "켜기" : "끄기"} · preload ${tls.hstsPreload && preloadAllowed ? "켜기" : "끄기"}`
                : "끄기"
            }
          >
            {" "}
            <FieldGroup id="tls-hsts" legend="HSTS">
              <SegmentedControl
                label="HSTS"
                value={tls.hstsEnabled ? "on" : "off"}
                onChange={(v) => tls.setHstsEnabled(v === "on")}
                options={[
                  { value: "off", label: "off" },
                  { value: "on", label: "on" },
                ]}
              />
            </FieldGroup>
            {tls.hstsEnabled ? (
              <>
                <Field label="HSTS max-age (초)" htmlFor="hsts-maxage">
                  <TextInput
                    type="number"
                    min={0}
                    value={tls.hstsMaxAge}
                    onChange={(e) => tls.setHstsMaxAge(e.target.value)}
                  />
                </Field>
                <FieldGroup id="tls-hsts-sub" legend="includeSubDomains">
                  <SegmentedControl
                    label="includeSubDomains"
                    value={tls.hstsSub ? "on" : "off"}
                    onChange={(v) => tls.setHstsSub(v === "on")}
                    options={[
                      { value: "off", label: "off" },
                      { value: "on", label: "on" },
                    ]}
                  />
                </FieldGroup>
                <FieldGroup
                  id="tls-hsts-preload"
                  legend="preload"
                  error={
                    preloadAllowed
                      ? undefined
                      : `preload는 max-age ≥ ${preloadMinMaxAge} + includeSubDomains일 때만 켤 수 있습니다. (브라우저 preload 목록 등재는 되돌리기 어려우니 신중히)`
                  }
                >
                  <SegmentedControl
                    label="preload"
                    value={tls.hstsPreload && preloadAllowed ? "on" : "off"}
                    onChange={(v) => tls.setHstsPreload(v === "on")}
                    options={[
                      { value: "off", label: "off" },
                      { value: "on", label: "on" },
                    ]}
                  />
                </FieldGroup>
              </>
            ) : null}
          </FormDisclosure>
          {uncoveredNames.length > 0 ? (
            <Banner
              tone="warning"
              title={
                <>
                  경고: server_name 중 인증서 도메인에 없는 이름이 있습니다:{" "}
                  {uncoveredNames.join(", ")}
                </>
              }
            >
              해당 이름으로 접속하면 브라우저 인증서 경고가 발생합니다.
              <p>
                인증서 도메인:{" "}
                {certificates
                  .find((c) => c.id === tls.certificateId)
                  ?.domains.join(", ")}
              </p>
              <Link
                href={`${certificateBaseHref}/${encodeURIComponent(tls.certificateId)}`}
              >
                인증서 보기
              </Link>
            </Banner>
          ) : null}
        </>
      ) : null}
    </div>
  );
}

export interface AccessSectionState {
  ipMode: "" | "whitelist" | "blacklist";
  setIpMode: (value: "" | "whitelist" | "blacklist") => void;
  ipEntries: string;
  setIpEntries: (value: string) => void;
  realIpMode: "" | "cloudflare" | "custom";
  setRealIpMode: (value: "" | "cloudflare" | "custom") => void;
  realIpRanges: string;
  setRealIpRanges: (value: string) => void;
  realIpHeader: RealIpHeader;
  setRealIpHeader: (value: RealIpHeader) => void;
  realIpRecursive: boolean;
  setRealIpRecursive: (value: boolean) => void;
}

/**
 * "접근 제어" tab: who may reach this server, and how the real client IP is
 * recovered when something else terminates the connection first.
 *
 * These two belong together: an IP allow list is only meaningful once nginx
 * knows the real visitor address, so getting the edge setting wrong silently
 * makes the allow list match the proxy instead of the visitor.
 */
function AccessFields({
  access,
  headerOptions,
  whitelistEmpty,
  customRangesEmpty,
  edgeWithoutCertificate,
  fieldErrors,
}: {
  access: AccessSectionState;
  headerOptions: readonly RealIpHeader[];
  /** 허용 목록 모드인데 항목이 하나도 없다 — 전원 차단이 된다. */
  whitelistEmpty: boolean;
  customRangesEmpty: boolean;
  /** 엣지 모드인데 인증서 미연결 — 오리진이 HTTP로 서빙된다. */
  edgeWithoutCertificate: boolean;
  fieldErrors?: Record<string, string>;
}) {
  return (
    <>
      <div className={formStyles.directiveGroup}>
        <div className={layout.sectionHead}>
          <span className={formStyles.directiveGroupTitle}>IP 접근 제어</span>
          <InfoTip label="IP 접근 제어 설명">
            서버 전체에 적용되는 IP allow/deny입니다.
          </InfoTip>
        </div>
        <p className={formStyles.directiveGroupNote}>
          (한 줄에 하나: IPv4/IPv6/CIDR)
        </p>
        <Field label="모드" htmlFor="ip-mode">
          <Select
            value={access.ipMode}
            onChange={(v) =>
              access.setIpMode(v as AccessSectionState["ipMode"])
            }
            options={[
              { value: "", label: "없음" },
              {
                value: "whitelist",
                label: "허용 목록 (나열 IP만 허용, 나머지 차단)",
              },
              { value: "blacklist", label: "차단 목록 (나열 IP 차단)" },
            ]}
            aria-label="IP 접근 제어 모드"
          />
        </Field>
        {access.ipMode ? (
          <Field
            label="IP / CIDR (한 줄에 하나)"
            htmlFor="ip-entries"
            error={
              [
                whitelistEmpty
                  ? "허용 목록이 비어 있으면 전원 차단됩니다 — 최소 1개를 입력하세요."
                  : null,
                fieldErrors?.ipAccess,
              ]
                .filter(Boolean)
                .join(" ") || undefined
            }
          >
            <TextArea
              mono
              value={access.ipEntries}
              onChange={(e) => access.setIpEntries(e.target.value)}
              placeholder={"127.0.0.1\n10.0.0.0/8"}
            />
          </Field>
        ) : null}
        {!access.ipMode && fieldErrors?.ipAccess ? (
          <Banner tone="error" role="alert" title={fieldErrors.ipAccess} />
        ) : null}
      </div>

      <div className={formStyles.directiveGroup}>
        <div className={layout.sectionHead}>
          <span className={formStyles.directiveGroupTitle}>
            엣지 / 프록시 뒤 배치 (실 IP 복원)
          </span>
          <InfoTip label="엣지 / 프록시 뒤 배치 (실 IP 복원) 설명">
            Cloudflare처럼 앞단에서 HTTPS를 대신 종단하는 서비스 뒤에 있을 때,
            실제 방문자 IP를 복원합니다 (access.log·IP 접근제어가 실 방문자
            기준으로 동작). 없음이면 직접 노출로 간주합니다.
          </InfoTip>
        </div>
        <Field label="엣지 모드" htmlFor="realip-mode">
          <Select
            value={access.realIpMode}
            onChange={(v) =>
              access.setRealIpMode(v as AccessSectionState["realIpMode"])
            }
            options={[
              { value: "", label: "없음 (직접 노출)" },
              {
                value: "cloudflare",
                label: "Cloudflare (내장 대역 · CF-Connecting-IP)",
              },
              { value: "custom", label: "직접 입력 (다른 CDN/LB)" },
            ]}
            aria-label="엣지 모드"
          />
        </Field>
        {access.realIpMode === "cloudflare" ? (
          <p className={formStyles.directiveGroupNote}>
            내장 Cloudflare IP 대역을 신뢰 프록시로 사용하고 실 IP 헤더는{" "}
            <code>CF-Connecting-IP</code>입니다. HTTPS 종단은 Cloudflare가
            처리합니다.
          </p>
        ) : null}
        {access.realIpMode === "custom" ? (
          <>
            <Field
              label="신뢰 프록시 IP / CIDR (한 줄에 하나)"
              htmlFor="realip-ranges"
              error={
                customRangesEmpty
                  ? "직접 입력 모드는 신뢰 대역이 최소 1개 필요합니다."
                  : undefined
              }
            >
              <TextArea
                mono
                value={access.realIpRanges}
                onChange={(e) => access.setRealIpRanges(e.target.value)}
                placeholder={"203.0.113.0/24\n2001:db8::/32"}
              />
            </Field>
            <Field label="실 IP 헤더" htmlFor="realip-header">
              <Select
                value={access.realIpHeader}
                onChange={(v) => access.setRealIpHeader(v as RealIpHeader)}
                options={headerOptions.map((h) => ({ value: h, label: h }))}
                aria-label="실 IP 헤더"
              />
            </Field>
            <FieldGroup id="realip-recursive" legend="real_ip_recursive">
              <SegmentedControl
                label="real_ip_recursive"
                value={access.realIpRecursive ? "on" : "off"}
                onChange={(v) => access.setRealIpRecursive(v === "on")}
                options={[
                  { value: "off", label: "off (권장)" },
                  { value: "on", label: "on" },
                ]}
              />
            </FieldGroup>
          </>
        ) : null}
        {edgeWithoutCertificate ? (
          <p className={formStyles.directiveGroupNote}>
            인증서 미연결 + 엣지 모드 → 오리진은 HTTP로 서빙되고 HTTPS는 엣지가
            종단합니다(정상). 오리진도 HTTPS로 받으려면 &quot;HTTPS&quot;
            설정에서 인증서(예: Cloudflare Origin CA를 Certificates에서
            가져오기)를 연결하세요.
          </p>
        ) : null}
        {fieldErrors?.realIp ? (
          <Banner tone="error" role="alert" title={fieldErrors.realIp} />
        ) : null}
      </div>
    </>
  );
}

export function ProtectView({
  certificates,
  certificateBaseHref,
  tls,
  access,
  preloadAllowed,
  uncoveredNames,
  fieldErrors,
  locations,
  onRoute,
}: {
  certificates: CertificateOption[];
  certificateBaseHref: string;
  tls: TlsSectionState;
  access: AccessSectionState;
  preloadAllowed: boolean;
  uncoveredNames: string[];
  fieldErrors?: Record<string, string>;
  locations: LocationInput[];
  onRoute: (index: number) => void;
}) {
  return (
    <>
      <Card
        id="server-form-https"
        title="TLS"
        info="발급 완료된 인증서를 연결하면 이 서버는 HTTPS로 서빙됩니다. 인증서는 Certificates 메뉴에서 발급합니다."
        errorCount={
          Object.keys(fieldErrors ?? {}).filter((key) =>
            /^(certificateId|tlsPort|tlsOptions|http2)/.test(key),
          ).length
        }
        actions={
          <Link
            href={certificateBaseHref}
            className={buttonClassName({ variant: "secondary", size: "sm" })}
          >
            인증서 관리
          </Link>
        }
      >
        <div className={formStyles.pfCategory}>
          {fieldErrors?.tlsOptions ? (
            <Banner tone="error" role="alert" title={fieldErrors.tlsOptions} />
          ) : null}
          <TlsFields
            certificates={certificates}
            certificateBaseHref={certificateBaseHref}
            tls={tls}
            preloadAllowed={preloadAllowed}
            preloadMinMaxAge={31536000}
            uncoveredNames={uncoveredNames}
            fieldErrors={fieldErrors}
          />
        </div>
      </Card>
      <Card
        id="server-form-access"
        title="접근 제어"
        info="HTTPS와 실제 방문자 IP를 기준으로 접속을 보호합니다. IP 접근 제어는 아래 실제 방문자 IP 복원을 기준으로 적용됩니다."
        errorCount={
          Object.keys(fieldErrors ?? {}).filter((key) =>
            /^(ipAccess|realIp)/.test(key),
          ).length
        }
      >
        <div className={formStyles.pfCategory}>
          <div className={formStyles.pfOverrides}>
            {locations.map((route, index) =>
              route.ipAccess ? (
                <Button
                  key={index}
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={() => onRoute(index)}
                >
                  {route.path}: IP{" "}
                  {route.ipAccess.mode === "whitelist" ? "허용" : "차단"}{" "}
                  {route.ipAccess.entries.length}개
                </Button>
              ) : null,
            )}
          </div>
          <AccessFields
            access={access}
            headerOptions={["CF-Connecting-IP", "X-Forwarded-For", "X-Real-IP"]}
            whitelistEmpty={
              access.ipMode === "whitelist" && !access.ipEntries.trim()
            }
            customRangesEmpty={
              access.realIpMode === "custom" && !access.realIpRanges.trim()
            }
            edgeWithoutCertificate={!!access.realIpMode && !tls.certificateId}
            fieldErrors={fieldErrors}
          />
        </div>
      </Card>
    </>
  );
}
