"use client";
import { useId } from "react";
import {
  Banner,
  Button,
  Field,
  Card,
  SegmentedControl,
  Select,
  Switch,
  TextInput,
} from "@repo/ui";
import type { CacheConfig, GzipConfig } from "@repo/presentation/nginx";
import {
  DEFAULT_CACHE_CONFIG,
  DEFAULT_GZIP_CONFIG,
} from "@repo/presentation/nginx/performance";
import type { DirectiveEditorState } from "@repo/presentation/nginx/directive-state";
import type { LocationInput } from "../server-form-vm";
import { DirectiveEditor, FormDisclosure } from "../directive-editor";
import formStyles from "@/components/reverse-proxy/server-form.module.css";
export function CommonView({
  cache,
  gzip,
  onCacheChange,
  onGzipChange,
  legacyGzip,
  serverDirectives,
  onServerDirectivesChange,
  locations,
  onRoute,
  fieldErrors,
}: {
  cache?: CacheConfig;
  gzip?: GzipConfig;
  onCacheChange: (value: CacheConfig) => void;
  onGzipChange: (value: GzipConfig | undefined) => void;
  legacyGzip: boolean;
  serverDirectives: DirectiveEditorState;
  onServerDirectivesChange: (value: DirectiveEditorState) => void;
  locations: LocationInput[];
  onRoute: (index: number) => void;
  fieldErrors?: Record<string, string>;
}) {
  const group = useId();
  const c = cache ?? DEFAULT_CACHE_CONFIG,
    g = gzip ?? DEFAULT_GZIP_CONFIG;
  const patchCache = (p: Partial<CacheConfig>) => onCacheChange({ ...c, ...p });
  const patchGzip = (p: Partial<GzipConfig>) => onGzipChange({ ...g, ...p });
  const exceptions = (key: "cacheEnabled" | "gzipEnabled") =>
    locations.map((route, index) =>
      route[key] !== undefined ? (
        <Button
          key={index}
          type="button"
          size="sm"
          variant="ghost"
          onClick={() => onRoute(index)}
        >
          {route.path}: {route[key] ? "켜기" : "끄기"}
        </Button>
      ) : null,
    );
  return (
    <>
      <Card
        id="server-form-cache"
        title="프록시 캐시"
        description="같은 요청에 원본 서버의 응답을 재사용합니다."
        errorCount={fieldErrors?.cache ? 1 : 0}
        off={!c.enabled && !fieldErrors?.cache}
        actions={
          <label className={formStyles.pfUseLabel}>
            사용
            <Switch
              label="서버 기본 캐시"
              checked={c.enabled}
              onCheckedChange={(enabled) => patchCache({ enabled })}
            />
          </label>
        }
      >
        <div className={formStyles.pfCategory}>
          <div className={formStyles.pfOverrides}>
            {exceptions("cacheEnabled")}
          </div>
          <FormDisclosure
            group={group}
            title="캐시 세부 설정"
            summary={`${c.maxSize} · 유효 ${c.valid} · 미사용 ${c.inactive} · stale ${c.useStale ? "켜기" : "끄기"} · lock ${c.lock ? "켜기" : "끄기"} · 상태 헤더 ${c.statusHeader ? "켜기" : "끄기"}`}
          >
            <div className={formStyles.pfFields}>
              <Field label="최대 저장 용량" htmlFor="cache-size">
                <Select
                  value={c.maxSize}
                  onChange={(maxSize) =>
                    patchCache({ maxSize: maxSize as CacheConfig["maxSize"] })
                  }
                  options={[
                    { value: "100m", label: "100 MB" },
                    { value: "500m", label: "500 MB" },
                    { value: "1g", label: "1 GB" },
                    { value: "5g", label: "5 GB" },
                  ]}
                  aria-label="최대 저장 용량"
                />
              </Field>
              <Field label="응답 유효기간" htmlFor="cache-valid">
                <Select
                  value={c.valid}
                  onChange={(valid) =>
                    patchCache({ valid: valid as CacheConfig["valid"] })
                  }
                  options={[
                    { value: "1m", label: "1분" },
                    { value: "10m", label: "10분" },
                    { value: "1h", label: "1시간" },
                    { value: "24h", label: "24시간" },
                  ]}
                  aria-label="응답 유효기간"
                />
              </Field>
              <Field label="미사용 캐시 보관 기간" htmlFor="cache-inactive">
                <Select
                  value={c.inactive}
                  onChange={(inactive) =>
                    patchCache({
                      inactive: inactive as CacheConfig["inactive"],
                    })
                  }
                  options={[
                    { value: "10m", label: "10분" },
                    { value: "60m", label: "1시간" },
                    { value: "24h", label: "24시간" },
                  ]}
                  aria-label="미사용 캐시 보관 기간"
                />
              </Field>
              {(
                [
                  ["useStale", "원본 오류 시 기존 캐시 사용"],
                  ["lock", "같은 응답의 중복 요청 대기"],
                  ["statusHeader", "캐시 상태 응답 헤더"],
                ] as const
              ).map(([key, label]) => (
                <Field key={key} label={label} htmlFor={`cache-${key}`}>
                  <Switch
                    label={label}
                    checked={c[key]}
                    onCheckedChange={(v) => patchCache({ [key]: v })}
                  />
                </Field>
              ))}
            </div>
            <p className={formStyles.directiveGroupNote}>
              로그인·분석 쿠키나 인증 헤더가 있는 요청과 원본 서버가 저장을
              금지한 응답은 캐시하지 않습니다. 정적 자원도 요청에 쿠키가 실리면
              캐시를 우회합니다. 유효기간은 원본 서버가 기간을 지정하지 않은
              200·301·302 응답에 적용됩니다.
            </p>
            <p className={formStyles.directiveGroupNote}>
              상태 헤더를 켜면 X-Cache-Status에서 HIT·MISS·BYPASS를 확인할 수
              있습니다. 캐시를 켠 경로는 응답 버퍼링도 켜져 있어야 합니다. 캐시
              파일은 컨테이너를 재생성하면 사라집니다.
            </p>
          </FormDisclosure>
          {fieldErrors?.cache ? (
            <Banner tone="error" role="alert" title={fieldErrors.cache} />
          ) : null}
        </div>
      </Card>
      <Card
        id="server-form-gzip"
        title="gzip"
        description="압축을 지원하는 클라이언트에 응답을 작게 전송합니다."
        errorCount={fieldErrors?.gzip ? 1 : 0}
        off={!g.enabled && !(legacyGzip && !gzip) && !fieldErrors?.gzip}
        actions={
          legacyGzip ? (
            <SegmentedControl
              label="서버 기본 gzip"
              value={!gzip ? "legacy" : g.enabled ? "on" : "off"}
              onChange={(v) =>
                v === "legacy"
                  ? onGzipChange(undefined)
                  : patchGzip({ enabled: v === "on" })
              }
              options={[
                { value: "legacy", label: "고급 설정 사용" },
                { value: "off", label: "끄기" },
                { value: "on", label: "켜기" },
              ]}
            />
          ) : (
            <label className={formStyles.pfUseLabel}>
              사용
              <Switch
                label="서버 기본 gzip"
                checked={g.enabled}
                onCheckedChange={(enabled) => patchGzip({ enabled })}
              />
            </label>
          )
        }
      >
        <div className={formStyles.pfCategory}>
          <div className={formStyles.pfOverrides}>
            {exceptions("gzipEnabled")}
          </div>
          <FormDisclosure
            group={group}
            title="압축 조건"
            summary={`${g.types === "text" ? "텍스트" : "텍스트 + 글꼴"} · 수준 ${g.compLevel} · 최소 ${Number.isFinite(g.minLength) ? g.minLength : "미입력"}바이트 · Vary ${g.vary ? "켜기" : "끄기"} · 프록시 ${g.proxied ? "켜기" : "끄기"}`}
          >
            <div className={formStyles.pfFields}>
              <Field label="압축 대상" htmlFor="gzip-types">
                <Select
                  value={g.types}
                  onChange={(types) =>
                    patchGzip({ types: types as GzipConfig["types"] })
                  }
                  options={[
                    {
                      value: "text",
                      label: "텍스트 (HTML·CSS·JS·JSON·XML·SVG)",
                    },
                    { value: "text-and-fonts", label: "텍스트 + 비압축 글꼴" },
                  ]}
                  aria-label="압축 대상"
                />
              </Field>
              <Field label="압축 수준" htmlFor="gzip-level">
                <Select
                  value={String(g.compLevel)}
                  onChange={(v) => patchGzip({ compLevel: Number(v) })}
                  options={Array.from({ length: 9 }, (_, i) => ({
                    value: String(i + 1),
                    label: String(i + 1),
                  }))}
                  aria-label="압축 수준"
                />
              </Field>
              <Field
                label="최소 응답 크기 (바이트)"
                htmlFor="gzip-min-length"
                hint="0~1,048,576바이트. Content-Length 응답 헤더가 있는 경우에만 적용합니다."
              >
                <TextInput
                  inputMode="numeric"
                  value={Number.isFinite(g.minLength) ? g.minLength : ""}
                  onChange={(e) =>
                    patchGzip({
                      minLength: e.target.value.trim()
                        ? Number(e.target.value)
                        : Number.NaN,
                    })
                  }
                />
              </Field>
              <Field label="Vary 응답 헤더" htmlFor="gzip-vary">
                <Switch
                  label="Vary 응답 헤더"
                  checked={g.vary}
                  onCheckedChange={(vary) => patchGzip({ vary })}
                />
              </Field>
              <Field label="프록시를 거친 요청도 압축" htmlFor="gzip-proxied">
                <Switch
                  label="프록시를 거친 요청도 압축"
                  checked={g.proxied}
                  onCheckedChange={(proxied) => patchGzip({ proxied })}
                />
              </Field>
            </div>
            <p className={formStyles.directiveGroupNote}>
              Vary 헤더는 중간 캐시가 압축 응답과 일반 응답을 구분하도록 합니다.
              프록시 요청 압축은 Via 요청 헤더가 있는 경우에도 적용합니다. 아래
              압축 조건은 모든 경로가 공유합니다.
            </p>
          </FormDisclosure>
          {fieldErrors?.gzip ? (
            <Banner tone="error" role="alert" title={fieldErrors.gzip} />
          ) : null}
        </div>
      </Card>
      <Card
        id="server-form-common"
        title="server 블록 디렉티브"
        info="경로에서 따로 정하지 않으면 이 서버 값을 사용합니다."
        errorCount={fieldErrors?.serverDirectives ? 1 : 0}
      >
        <div className={formStyles.pfCategory}>
          {fieldErrors?.serverDirectives ? (
            <Banner
              tone="error"
              role="alert"
              title={fieldErrors.serverDirectives}
            />
          ) : null}
          <div className={formStyles.pfOverrides}>
            {locations.map((route, index) => {
              const values = [
                route.directives.bodySize
                  ? `본문 ${route.directives.bodySize}`
                  : "",
                ...Object.entries(route.directives.timeouts)
                  .filter(([, value]) => value)
                  .map(([key, value]) => `${key} ${value}`),
              ].filter(Boolean);
              return values.length ? (
                <Button
                  key={index}
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={() => onRoute(index)}
                >
                  {route.path}: {values.join(" · ")}
                </Button>
              ) : null;
            })}
          </div>
          <DirectiveEditor
            group={group}
            context="server"
            value={serverDirectives}
            onChange={onServerDirectivesChange}
          />
        </div>
      </Card>
    </>
  );
}
