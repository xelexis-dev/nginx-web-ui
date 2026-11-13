import { describe, expect, it } from "vitest";
import { DEFAULT_LOCATION_HEADERS } from "@repo/presentation/nginx/directive-state";
import { carryDirectives } from "@repo/presentation/nginx/directive-carry";
import {
  toCertificateOptions,
  toServerFormValues,
  newLocationInput,
  serializeLocations,
  toLocationSubmission,
  type CertificateSourceRow,
  type ServerFormSourceRow,
} from "./server-form-vm";

/**
 * 폼 초기값과 후보 목록을 만드는 **순수 함수 둘**. 로컬 수정 화면과 노드 수정
 * 화면이 이 둘을 공유하므로, 여기가 갈리면 같은 서버가 게이트웨이와 노드에서
 * 다르게 열린다.
 *
 * 훅도 DOM 도 없어 그냥 부르면 된다(`components/node-error.test.tsx` 와 같은
 * 관용) — 그래서 이 계획에서 **화면이 아니라 값**을 직접 붙들 수 있는 몇 안 되는
 * 자리다.
 */

const UPSTREAMS = [
  { id: "up-1", name: "node-pool" },
  { id: "up-2", name: "other-pool" },
];

describe("per-route conversion and directive carry", () => {
  it("converts an untouched legacy direct server without materialising implicit headers", () => {
    const values = toServerFormValues(serverRow({ config: { proxyPass: "http://origin.test:80/path/", resolveAtRuntime: true } }), UPSTREAMS);
    expect(values).toHaveProperty("convertedFromSingle", true);
    expect(values.locations).toHaveLength(1);
    expect(values.locations?.[0]).toMatchObject({ path: "/", mode: "direct", proxyPass: "http://origin.test:80/path/", resolveAtRuntime: true, originalDirectives: null });
    expect(values.locations?.[0]?.directives.headers).toEqual([]);
  });

  it("carries a single upstream's ID and ordered directives without route overrides", () => {
    const original = [{ name: "proxy_read_timeout", args: ["300s"] }, { name: "proxy_set_header", args: ["Host", "$host"] }];
    const values = toServerFormValues(serverRow({ upstreamId: "up-1", config: { locationDirectives: original, cache: { enabled: true } } }), UPSTREAMS);
    expect(values.locations?.[0]).toMatchObject({ path: "/", mode: "upstream", upstreamId: "up-1", originalDirectives: original });
    expect(values.locations?.[0]).not.toHaveProperty("cacheEnabled");
    expect(values.locations?.[0]).not.toHaveProperty("ipAccess");
  });

  it("prefers the canonical ID over a stale derived name or URL", () => {
    const values = toServerFormValues(serverRow({ config: { locations: [{ path: "/", upstreamId: "up-2", upstreamName: "old-name", proxyPass: "http://node-pool", directives: [] }] } }), UPSTREAMS);
    expect(values.locations?.[0]).toMatchObject({ mode: "upstream", upstreamId: "up-2", proxyPass: "", originalDirectives: [] });
  });

  it("shows a dangling ID as an empty upstream selection with an actionable error", () => {
    const values = toServerFormValues(serverRow({ config: { locations: [{ path: "/missing", upstreamId: "foreign-id", upstreamName: "stale" }] } }), UPSTREAMS);
    expect(values.locations?.[0]).toMatchObject({ mode: "upstream", upstreamId: "", upstreamError: "연결된 Upstream을 찾을 수 없습니다(삭제됐거나 이 서버 범위에 없음). 다른 Upstream을 고르거나 직접 주소로 바꾸세요." });
  });

  it("does not recover an empty ID by silently using a stale URL", () => {
    const values = toServerFormValues(serverRow({ config: { locations: [{ path: "/", upstreamId: "", proxyPass: "http://node-pool" }] } }), UPSTREAMS);
    expect(values.locations?.[0]).toMatchObject({ mode: "upstream", upstreamId: "", proxyPass: "", upstreamError: expect.any(String) });
  });
});

describe("per-route form submission", () => {
  it.each([undefined, [], [{ name: "add_header", args: ["X-First", "it's quoted"] }, { name: "proxy_read_timeout", args: ["300s"] }, { name: "proxy_set_header", args: ["Host", "$host"] }]].map((directives) => ({ directives })))(
    "carries single/route arrays and absence through real JSON without adding defaults: %j", ({ directives }) => {
      for (const perRoute of [false, true]) {
        const stored = { ...(directives !== undefined ? { directives } : {}) };
        const config = perRoute
          ? { locations: [{ path: "/", proxyPass: "http://origin.test:80/a/", ...stored }] }
          : { proxyPass: "http://origin.test:80/a/", ...(directives !== undefined ? { locationDirectives: directives } : {}) };
        const values = toServerFormValues(serverRow({ config }), UPSTREAMS);
        const [wire] = JSON.parse(serializeLocations(values.locations!));
        expect(wire).toEqual({ path: "/", mode: "direct", proxyPass: "http://origin.test:80/a/", directives: directives ?? null });
        expect(values.convertedFromSingle).toBe(!perRoute);
      }
    },
  );

  it("carries the server set independently; editing one route does not rewrite another set", () => {
    const original = [{ name: "add_header", args: ["X-First", "one"] }, { name: "proxy_read_timeout", args: ["300s"] }];
    const values = toServerFormValues(serverRow({ config: { serverDirectives: original, locations: [
      { path: "/", proxyPass: "http://origin.test", directives: original },
      { path: "/other", proxyPass: "http://other.test", directives: original },
    ] } }), UPSTREAMS);
    values.locations![0]!.directives.timeouts.read = "10s";
    expect(toLocationSubmission(values.locations![0]!).directives).toBe(values.locations![0]!.directives);
    expect(toLocationSubmission(values.locations![1]!).directives).toBe(original);
    expect(carryDirectives(values.originalServerDirectives, values.serverDirectives!)).toBe(original);
  });

  it.each(["Upstream-selected", "direct-entered"])("converts an exact-name %s literal by policy", () => {
    const values = toServerFormValues(serverRow({ config: { locations: [{ path: "/", proxyPass: "http://node-pool" }] } }), UPSTREAMS);
    expect(JSON.parse(serializeLocations(values.locations!))).toEqual([{ path: "/", mode: "upstream", upstreamId: "up-1", directives: null }]);
  });

  it.each(["http://node-pool/", "http://node-pool:80", "https://node-pool", "http://Node-pool", "http://node-pool/path"])("keeps near-miss %s byte-for-byte as direct", (proxyPass) => {
    const values = toServerFormValues(serverRow({ config: { locations: [{ path: "/", proxyPass }] } }), UPSTREAMS);
    expect(JSON.parse(serializeLocations(values.locations!))).toEqual([{ path: "/", mode: "direct", proxyPass, directives: null }]);
  });

  it("seeds new routes, but a deliberately emptied new route remains explicit editor state", () => {
    const location = newLocationInput();
    expect(location.directives.headers).toEqual(DEFAULT_LOCATION_HEADERS);
    expect(toLocationSubmission(location).directives).toBe(location.directives);
    location.directives.headers = [];
    expect(toLocationSubmission(location).directives).toBe(location.directives);
  });

  it("emits DNS only for true direct flags and omits read-only/carry fields", () => {
    const values = toServerFormValues(serverRow({ config: { locations: [
      { path: "/", upstreamId: "up-1", upstreamName: "node-pool", resolveAtRuntime: true, cacheEnabled: false },
      { path: "/dns", proxyPass: "http://origin.test", resolveAtRuntime: true },
      { path: "/static", proxyPass: "http://static.test", resolveAtRuntime: false },
    ] } }), UPSTREAMS);
    const wire = JSON.parse(serializeLocations(values.locations!));
    expect(wire[0]).toEqual({ path: "/", mode: "upstream", upstreamId: "up-1", directives: null, cacheEnabled: false });
    expect(wire[1].resolveAtRuntime).toBe(true);
    expect(wire[2]).not.toHaveProperty("resolveAtRuntime");
    expect(serializeLocations(values.locations!)).not.toMatch(/upstreamName|originalDirectives|upstreamError/);
  });
});

describe("toServerFormValues — cache and gzip boundaries", () => {
  it("keeps legacy gzip in the advanced editor without creating structured gzip", () => {
    const values = toServerFormValues(serverRow({ config: {
      serverDirectives: [{ name: "gzip", args: ["on"] }],
    } }), []);
    expect(values.gzip).toBeUndefined();
    expect(values.serverDirectives?.advanced).toContain("gzip on;");
  });

  it.each([
    { cache: { enabled: "true" } },
    { gzip: { minLength: null } },
    { locations: [{ path: "/", proxyPass: "http://origin.test", cacheEnabled: "off" }] },
  ])("rejects malformed remote performance configuration at the viewmodel boundary", (config) => {
    expect(() => toServerFormValues(serverRow({ config }), [])).toThrow();
  });
});

function serverRow(extra: Partial<ServerFormSourceRow> = {}): ServerFormSourceRow {
  return {
    id: "srv-1",
    serverName: "a.example.com",
    listen: 80,
    enabled: true,
    upstreamId: null,
    certificateId: null,
    config: {},
    ...extra,
  };
}

describe("toCertificateOptions", () => {
  const ROWS: CertificateSourceRow[] = [
    { id: "cert-1", domains: ["a.example.com", "www.a.example.com"], staging: false, status: "issued" },
    { id: "cert-2", domains: ["b.example.com"], staging: true, status: "pending" },
    { id: "cert-3", domains: ["c.example.com"], staging: false, status: "failed" },
  ];

  /**
   * **노드에서 실제로 일하는 필터다.** 로컬은 SQL 이 이미 issued 만 남기지만,
   * 노드 화면은 `/api/v1/certificates` 목록 전체를 받는다 — 빼면 발급 전·실패한
   * 인증서가 후보로 뜨고, 고르면 노드가 저장은 하지만 `applyAll` 이 TLS 블록을
   * 쓰지 않아 **HTTPS 가 안 되는데 화면은 「연결됨」**이 된다.
   */
  it("발급 완료가 아닌 인증서는 후보에서 뺀다", () => {
    expect(toCertificateOptions(ROWS).map((c) => c.id)).toEqual(["cert-1"]);
  });

  it("라벨은 첫 도메인이고, 도메인이 없으면 id 로 떨어진다", () => {
    const options = toCertificateOptions([
      ...ROWS,
      { id: "cert-4", domains: [], staging: false, status: "issued" },
      // jsonb 컬럼이라 배열이 아닐 수 있다 — 그때도 던지지 않는다.
      { id: "cert-5", domains: null, staging: true, status: "issued" },
    ]);

    expect(options).toEqual([
      { id: "cert-1", label: "a.example.com", staging: false, domains: ["a.example.com", "www.a.example.com"] },
      { id: "cert-4", label: "cert-4", staging: false, domains: [] },
      { id: "cert-5", label: "cert-5", staging: true, domains: [] },
    ]);
  });

  /**
   * 별칭은 라벨 **앞**에 서고 대표 도메인은 괄호로 남는다 — 사람이 붙인 이름으로
   * 고르되, 어느 도메인 인증서인지는 계속 보여야 한다.
   */
  it("별칭이 있으면 「별칭 (대표 도메인)」이 라벨이다", () => {
    const [option] = toCertificateOptions([
      { id: "cert-1", domains: ["a.example.com", "www.a.example.com"], staging: false, status: "issued", alias: "운영 API" },
    ]);

    expect(option?.label).toBe("운영 API (a.example.com)");
  });

  it("별칭이 있는데 도메인이 없으면 「별칭 (id)」다", () => {
    const [option] = toCertificateOptions([{ id: "cert-9", domains: [], staging: false, status: "issued", alias: "임시" }]);

    expect(option?.label).toBe("임시 (cert-9)");
  });

  /**
   * 별칭 필드를 모르는 **구 노드**의 응답에는 `alias` 키 자체가 없고, 별칭을 넣지
   * 않은 행은 `null` 이다. 둘 다 기존 라벨 그대로여야 한다 — 여기서 던지면 구 노드의
   * 서버 폼이 통째로 카드가 된다.
   */
  it("별칭이 없거나(구 노드) null 이면 기존 라벨 그대로다", () => {
    const options = toCertificateOptions([
      { id: "cert-1", domains: ["a.example.com"], staging: false, status: "issued" },
      { id: "cert-2", domains: ["b.example.com"], staging: false, status: "issued", alias: null },
    ]);

    expect(options.map((o) => o.label)).toEqual(["a.example.com", "b.example.com"]);
  });
});

describe("toServerFormValues — 대상 되돌리기", () => {
  /**
   * 저장되는 것은 해석된 문자열(`http://<name>`)이라, 폼을 열 때 그것을 다시
   * upstream id 로 되돌려야 한다(`buildServerRow` 문서).
   */
  it("저장된 http://<name> 을 알려진 upstream 이면 upstream 모드로 되돌린다", () => {
    const values = toServerFormValues(
      serverRow({ config: { locations: [{ path: "/", proxyPass: "http://node-pool", directives: [] }] } }),
      UPSTREAMS,
    );

    expect(values.locations).toMatchObject([
      { path: "/", mode: "upstream", upstreamId: "up-1", proxyPass: "", directives: expect.anything(), originalDirectives: [], resolveAtRuntime: false },
    ]);
  });

  /**
   * **이 검사가 「후보 목록은 그 서버와 같은 출처여야 한다」를 값으로 붙든다.**
   * 노드 화면에 게이트웨이 자신의 upstream 목록을 실으면 이름이 안 맞아
   * upstream 으로 저장된 location 이 **직접 proxy_pass 로 열리고**, 사용자가
   * 저장하는 순간 그 값이 굳는다 — 화면은 멀쩡히 뜨고 타입도 통과한다.
   */
  it("목록에 없는 이름이면 직접 proxy_pass 로 열린다 — 남의 목록을 주면 이렇게 된다", () => {
    const values = toServerFormValues(
      serverRow({ config: { locations: [{ path: "/", proxyPass: "http://node-pool", directives: [] }] } }),
      // 게이트웨이 자신의 목록을 흉내낸다 — id 는 같은데 이름이 다르다.
      [{ id: "up-1", name: "gateway-pool" }],
    );

    expect(values.locations?.[0]).toMatchObject({ mode: "direct", upstreamId: "", proxyPass: "http://node-pool" });
  });

  it("upstream 이 연결된 단일 모드에서는 proxyPass 를 폼에 싣지 않는다", () => {
    const values = toServerFormValues(
      serverRow({ upstreamId: "up-2", config: { proxyPass: "http://127.0.0.1:9000" } }),
      UPSTREAMS,
    );

    expect(values.upstreamId).toBe("up-2");
    expect(values.proxyPass).toBeNull();
  });

  it("upstream 이 없으면 저장된 proxy_pass 를 그대로 싣는다", () => {
    const values = toServerFormValues(serverRow({ config: { proxyPass: "http://127.0.0.1:9000" } }), UPSTREAMS);

    expect(values.upstreamId).toBeNull();
    expect(values.proxyPass).toBe("http://127.0.0.1:9000");
  });

  it("locations 가 비어 있으면 기존 단일 대상을 / 경로 하나로 연다", () => {
    const values = toServerFormValues(serverRow({ config: { locations: [], proxyPass: "http://origin.test" } }), UPSTREAMS);
    expect(values.locations).toMatchObject([{ path: "/", proxyPass: "http://origin.test" }]);
    expect(values.convertedFromSingle).toBe(true);
  });
});

describe("toServerFormValues — stored directive presence", () => {

  // Absence is carried to the API as explicit null; never seed headers for legacy rows.
  it("directives 가 없는 옛 location 은 빈 편집 상태와 누락 표시로 연다", () => {
    const values = toServerFormValues(
      serverRow({ config: { locations: [{ path: "/", proxyPass: "http://127.0.0.1:3000" }] } as never }),
      UPSTREAMS,
    );

    expect(values.locations?.[0]?.directives.headers).toEqual([]);
    expect(values.locations?.[0]?.originalDirectives).toBeNull();
  });

  /**
   * 반대로 **빈 배열로 저장된 것은 빈 채로 연다**(WYSIWYG) — 여기서도 씨를
   * 뿌리면 사용자가 일부러 지운 헤더가 저장할 때마다 되살아난다.
   */
  it("빈 배열로 저장된 location 디렉티브는 빈 채로 연다", () => {
    const values = toServerFormValues(serverRow({ config: { locationDirectives: [] } }), UPSTREAMS);

    expect(values.locationDirectives?.headers).toEqual([]);
  });

  it("locationDirectives 자체가 없어도 암묵 헤더를 명시적으로 만들지 않는다", () => {
    expect(toServerFormValues(serverRow(), UPSTREAMS).locationDirectives?.headers).toEqual([]);
  });
});

/**
 * **이 함수가 「읽지 않고 그대로 폼에 넘기던」 값들.** 그냥 통과시키면 어긋난
 * 모양이 `loadNodePageAll` 의 try/catch **밖**, 폼 렌더 도중에 터진다 — 하드
 * 로드에서는 §9 카드가 아니라 빈 화면이다(`LoadNodePageAllArgs.toViewModel`
 * 문서). 폼이 파고드는 깊이(`entries.join("\n")`)로 여기서 읽으면 그 실패가
 * 관문 안으로 들어와 카드가 된다.
 */
describe("toServerFormValues — 그대로 넘기던 값을 관문 안에서 읽는다", () => {
  it("서버 레벨 ipAccess.entries 가 배열이 아니면 여기서 던진다", () => {
    const row = serverRow({ config: { ipAccess: { mode: "whitelist", entries: "10.0.0.1" } } as never });

    // 문자열은 `.length`·`.slice()` 로는 안 걸린다 — 폼이 실제로 부르는 `.join`
    // 이어야 잡힌다(그래서 「렌더와 같은 깊이」다).
    expect(() => toServerFormValues(row, UPSTREAMS)).toThrow();
  });

  it("realIp.trustedRanges 가 배열이 아니면 여기서 던진다", () => {
    const row = serverRow({ config: { realIp: { preset: "custom", trustedRanges: 42 } } as never });

    expect(() => toServerFormValues(row, UPSTREAMS)).toThrow();
  });

  /** location 마다도 같은 필드가 있다 — 폼은 `form-sections/target.tsx:204` 에서 같은 호출을 한다. */
  it("location 의 ipAccess.entries 도 읽는다", () => {
    const row = serverRow({
      config: {
        locations: [{ path: "/", proxyPass: "http://node-pool", directives: [], ipAccess: { mode: "blacklist", entries: {} } }],
      } as never,
    });

    expect(() => toServerFormValues(row, UPSTREAMS)).toThrow();
  });

  /**
   * `trustedRanges` 는 **없어도 되는 필드**다(`preset: "cloudflare"` 에는 아예
   * 없다) — 읽기가 그것까지 막으면 정상 설정이 카드가 된다.
   */
  it("없는 것은 없는 대로 통과한다 — 읽기가 정상 설정을 막지 않는다", () => {
    const row = serverRow({
      config: {
        ipAccess: { mode: "whitelist", entries: ["10.0.0.0/8"] },
        realIp: { preset: "cloudflare" },
      } as never,
    });

    const values = toServerFormValues(row, UPSTREAMS);
    expect(values.ipAccess).toEqual({ mode: "whitelist", entries: ["10.0.0.0/8"] });
    expect(values.realIp).toEqual({ preset: "cloudflare" });
  });
});

describe("toServerFormValues — TLS 기본값", () => {
  it("저장된 값이 없으면 443 / HTTP 리다이렉트 켬으로 연다", () => {
    const values = toServerFormValues(serverRow({ certificateId: "cert-1" }), UPSTREAMS);

    expect(values.certificateId).toBe("cert-1");
    expect(values.tlsPort).toBe(443);
    expect(values.redirectHttp).toBe(true);
  });

  /**
   * `redirectHttp` 는 **`false` 만 꺼진 것**이다(`!== false`). 저장된 `false` 를
   * 못 읽으면 폼이 켜진 채로 열리고, 저장하는 순간 운영자가 일부러 꺼둔 HTTP →
   * HTTPS 리다이렉트가 되살아난다.
   */
  it("저장된 tlsPort·redirectHttp=false 를 그대로 읽는다", () => {
    const values = toServerFormValues(
      serverRow({ certificateId: "cert-1", config: { tlsPort: 8443, redirectHttp: false } as never }),
      UPSTREAMS,
    );

    expect(values.tlsPort).toBe(8443);
    expect(values.redirectHttp).toBe(false);
  });
});

it("round-trips runtime DNS for direct targets while named references defer to the group", () => {
  const direct = toServerFormValues(serverRow({ config: {
    proxyPass: "http://px-web:3001", resolveAtRuntime: true,
    locations: [
      { path: "/direct", proxyPass: "http://px-web:3001", resolveAtRuntime: true },
      { path: "/group", proxyPass: "http://node-pool", resolveAtRuntime: true },
    ],
  } }), UPSTREAMS);
  expect(direct.resolveAtRuntime).toBe(true);
  expect(direct.locations?.map(location => location.resolveAtRuntime)).toEqual([true, false]);
  expect(toServerFormValues(serverRow({ upstreamId: "up-1", config: { resolveAtRuntime: true } }), UPSTREAMS).resolveAtRuntime).toBe(false);
  expect(toServerFormValues(serverRow(), UPSTREAMS).resolveAtRuntime).toBe(false);
});

it("rejects malformed remote runtime DNS booleans at the viewmodel boundary", () => {
  expect(() => toServerFormValues(serverRow({ config: { resolveAtRuntime: "true" } }), [])).toThrow();
  expect(() => toServerFormValues(serverRow({ upstreamId: "up-1", config: { resolveAtRuntime: "true" } }), UPSTREAMS)).toThrow();
  expect(() => toServerFormValues(serverRow({ config: { locations: [{ path: "/", proxyPass: "http://px-web:3001", resolveAtRuntime: 1 }] } }), [])).toThrow();
});

it("carries an opaque row version through the shared local/node edit converter", () => {
  const row = { ...serverRow(), version: "9007199254740993" };
  expect(toServerFormValues(row, UPSTREAMS)).toHaveProperty("version", row.version);
  expect(toServerFormValues(serverRow(), UPSTREAMS)).not.toHaveProperty("version", expect.anything());
});
