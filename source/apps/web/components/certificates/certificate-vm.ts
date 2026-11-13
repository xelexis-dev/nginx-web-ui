/** Safe issuance-log fields used by certificate presentation. */
type IssueLogEntry = { at: string; level: "info" | "error"; msg: string };
import type { NodeActionState } from "@/lib/node-action-state";

/**
 * 인증서 화면(목록·상세)이 로컬·노드 양쪽에서 공유하는 **뷰모델과 그 변환**.
 *
 * 값 import 없이 안전한 API 응답을 화면용 값으로 변환한다.
 * 노드 응답의 중첩 값도 읽기 단계에서 확인해, 모양이 어긋나면
 * `loadNodePage*` 의 오류 카드 경로로 전달한다.
 *
 * **⚠ 그런데 「컨테이너 깊이」에서 멈추면 절반만 닫힌다.** `join` 은 원소를
 * 검사하지 않고 문자열로 만들고, `map` 은 배열인지만 본다 — 그 뒤 화면이
 * **원소를 그대로 React 자식으로** 넘기는 자리가 남는다(`domains[0]`,
 * `issueLog[].msg`, TXT 세 문자열, `failureCount`, `lastError`,
 * 연결 서버의 `serverName`·`listen`, `statusLabel` 이 모르는 값을 그대로
 * 되돌리는 갈래). 거기에 객체가 오면 렌더 도중 `Objects are not valid as a
 * React child` 로 터진다 — **이 파일이 막으려던 바로 그 부류다.**
 * 그래서 원소 깊이는 `readText`/`readNullableText`/`readNumber` 로 읽는다.
 *
 * 대체값으로 접으면 어긋난 API 응답이 「도메인 없음」·「로그 없음」으로
 * 조용히 그려진다. 따라서 잘못된 값을 발견하면 던진다.
 * 아래 세 함수가 `if` 를 쓰지만 **분기해서 다른 값을 내는 게 아니라 던진다** —
 * 던짐이 곧 §9 카드이고 렌더 경로는 하나도 늘지 않는다.
 */

/**
 * React 자식으로 그려질 문자열 하나를 **읽는다**. 어긋나면 던진다.
 *
 * `join`/`replace`/`toISOString` 처럼 **저절로 던지는 호출이 없는 자리**를 위한
 * 것이다 — 값을 그대로 넘기는 자리에는 옮겨 담기밖에 없어서, 읽으려면 타입을
 * 직접 봐야 한다. `field` 는 던질 때만 쓰이고 화면에는 안 나간다(§9 카드 문구는
 * 고정이다) — 스택 추적과 검사에서 어느 자리인지 알아보려는 것이다.
 */
export function readText(value: string, field: string): string {
  if (typeof value !== "string") throw new TypeError(`${field}: 문자열이 아닙니다.`);
  return value;
}

/** `readText` 의 nullable 판. `null` 은 계약이 정한 값이라 그대로 통과시킨다. */
export function readNullableText(value: string | null, field: string): string | null {
  return value === null ? null : readText(value, field);
}

/**
 * 별칭 읽기. `readNullableText` 와 **한 가지가 다르다** — `undefined` 도 「없음」으로
 * 통과한다. 별칭을 모르는 **구 노드**의 응답에는 키 자체가 없고, 그것을 던지면
 * 그 노드의 인증서 화면이 통째로 §9 카드가 된다. 다른 필드에는 이 관용을 쓰지
 * 마라 — 「빠진 필드는 통과하지 않는다」(`readInstant` 문서)가 기본 규칙이다.
 */
export function readAlias(value: string | null | undefined, field: string): string | null {
  return value === undefined ? null : readNullableText(value, field);
}

/**
 * `source` 를 관문에서 읽는다. **이 값은 화면에 그려지지 않는다** — 그래서 위
 * 함수들과 부류가 다르다. 여기서 읽는 이유는 **분기를 고르는 값**이기 때문이다:
 * 수정 화면이 `source !== "acme"` 로 교체 화면 리다이렉트를 정한다. 그냥
 * 통과시키면 노드가 이상한 값을 줬을 때 그 비교가 참이 되어 **§9 카드 대신
 * 멀쩡해 보이는 다른 화면**이 뜬다 — 실패가 실패로 안 보이는 자리다.
 */
export function readCertificateSource(value: CertificateSource, field: string): CertificateSource {
  if (value !== "acme" && value !== "manual") throw new TypeError(`${field}: 모르는 source 입니다.`);
  return value;
}

/**
 * React 자식으로 그려질 숫자 하나. **`NaN` 은 막지 않는다** — 그건 「NaN」으로
 * 그려질 뿐 렌더를 깨뜨리지 않아서, 이 함수가 닫으려는 부류가 아니다.
 */
export function readNumber(value: number, field: string): number {
  if (typeof value !== "number") throw new TypeError(`${field}: 숫자가 아닙니다.`);
  return value;
}

/** DNS-01 챌린지에서 사람이 DNS 에 넣어야 하는 TXT 한 줄. */
export interface DnsChallengeRecordView {
  domain: string;
  recordName: string;
  recordValue: string;
}

/**
 * 「전파 확인 후 계속」 액션이 패널에 돌려주는 상태.
 *
 * **`NodeActionState` 에 두 필드가 더 붙는다.** `notPropagated` 는 **실패가
 * 아니다** — 노드가 **200 + 플래그**로 낸다(`api/v1/certificates/[id]/dns-challenge/route.ts`:
 * 「TXT 가 아직 안 보이는 것은 오류가 아니라 더 기다리라는 뜻이다」). 그래서
 * `nodeError`(§9 카드)로 접으면 안 되고, 「어느 도메인이 아직인지」를 함께
 * 보여주면서 다시 누를 수 있어야 한다.
 *
 * **로컬 `ActionState` 액션이 그대로 들어간다** — 추가 필드가 전부 optional
 * 이라 양방향 대입이 성립한다(`NodeFormAction` 문서와 같은 원리).
 *
 * **왜 이 파일에 있나:** 값 의존이 0인 모듈이라(이 파일 헤더) 서버 액션도
 * `"use client"` 패널도 안전하게 들여올 수 있다. 패널 파일에 두면 서버 쪽
 * `actions.ts` 가 클라이언트 모듈을 가리키게 되고, 누군가 `import type` 을
 * 빠뜨리는 순간 그 사슬이 번들로 새어 들어간다(`lib/node-action-state.ts` 가
 * 따로 있는 것과 같은 이유).
 */
export interface DnsChallengeState extends NodeActionState {
  /** DNS-01: TXT 가 아직 안 보여 CA 검증을 요청하지 않았음. **오류가 아니다.** */
  notPropagated?: boolean;
  /** 아직 안 보이는 도메인들. */
  pending?: string[];
}

export type DnsChallengeAction = import("@/lib/use-api-action").FormAction<DnsChallengeState>;

/**
 * 진행 중인 DNS-01 주문이 **어느 단계인지**. 갈래는 둘뿐이고, **로컬과 노드가
 * 같은 값(`submittedAt`)으로 가른다** — 노드는 그것을 `dnsSubmittedAt` 으로
 * 받는다(`api/v1/certificates/view.ts`, 1.16.0).
 *
 * - `awaiting-txt` — 사용자가 TXT 를 넣고 「계속」을 누르기 전.
 * - `verifying` — 눌렀고 CA 검증이 백그라운드로 돈다.
 *
 * **이 둘을 `status` 로 가르려 하지 마라.** 최초 발급만 상태를 옮기고
 * (`awaiting-dns` → `issuing`), **갱신은 서빙 중인 인증서를 지키려고 두 단계
 * 모두 `issued` 를 유지한다**(`lib/acme-service.ts` 의 `submitDnsChallenge`).
 * 그래서 갱신 중에는 `status` 가 단계를 말해 주지 않는다.
 */
export type DnsOrderPhase = "awaiting-txt" | "verifying";

/**
 * 진행 중인 DNS-01 주문. 주문이 없으면 이 값 자체가 `null` 이다.
 *
 * **TXT 안내는 그것을 그리는 단계에만 실린다.** 검증 중에는 패널이 뜨지 않으므로
 * (`certificate-detail.tsx` 의 `dnsPanel` 은 `awaiting-txt` 에서만 채워진다)
 * 그 단계에 `records` 를 실으면 **아무도 안 그리는 값을 읽게 된다** — 로컬
 * 상세는 추출 전에도 `awaitingTxt` 갈래 안에서만 `records` 를 만졌다.
 */
export type DnsOrderVM =
  | { phase: "awaiting-txt"; records: DnsChallengeRecordView[] }
  | { phase: "verifying" };

export type CertificateStatus = "issuing" | "issued" | "failed" | "awaiting-dns";
export type CertificateSource = "acme" | "manual";
export type CertificateChallengeType = "http-01" | "dns-01";

const STATUS_LABEL: Record<string, string> = {
  issuing: "발급 중",
  issued: "발급 완료",
  failed: "실패",
  "awaiting-dns": "TXT 대기",
};

/**
 * 상태 배지·상세 상태 행의 문구. **한 벌뿐이어야 한다** — 추출 전에는 목록과
 * 상세가 같은 표를 각각 들고 있었다(`(app)/certificates/page.tsx:10-15` 와
 * `[id]/page.tsx:14-19`). 갈리면 같은 인증서가 두 화면에서 다른 상태로 읽힌다.
 */
export function statusLabel(status: string): string {
  // **`?? status` 가 원소 깊이의 구멍이다** — 모르는 값을 그대로 되돌리고 화면이
  // 그것을 React 자식으로 그린다. 이 함수는 **렌더 중에** 불리므로 여기서
  // 던지면 관문 밖이다 → 그래서 화면들은 `status` 를 `toViewModel` 안에서
  // `readText` 로 읽어 넣는다(노드 목록·상세 페이지). 여기 캐스트는 그 계약을
  // 적어두는 자리다.
  return STATUS_LABEL[status] ?? status;
}

/** 만료까지 남은 일수. `null` 은 「만료일 없음」(발급 전·실패)이다. */
export function daysLeft(expiresAt: Date | null): number | null {
  if (!expiresAt) return null;
  return Math.floor((expiresAt.getTime() - Date.now()) / (24 * 60 * 60 * 1000));
}

/** 상세의 시각 표기(`2026-08-29 12:34:56`). 값이 없으면 `—`. */
export function fmtInstant(d: Date | null): string {
  return d ? d.toISOString().replace("T", " ").slice(0, 19) : "—";
}

/**
 * 화면이 보여주는 도메인 세 가지. **`string[]` 을 그대로 실어 나르지 않는 것이
 * 요점이다** — 그러면 `[0]`·`.length`·`join` 이 전부 렌더 시점의 읽기가 되어
 * 관문 밖으로 샌다. 여기서 한 번 만들어 두면 그 읽기가 관문 안에서 일어난다.
 */
export interface DomainsVM {
  /** 대표 도메인(목록 행·상세 제목). 비어 있으면 화면이 자기 대체 문구를 고른다. */
  primary: string | null;
  /** 전부 — 상세의 「도메인」 행. */
  all: string;
  /** 대표 말고 몇 개 더 있는지 — 목록의 「외 N」. */
  extra: number;
}

/**
 * `map` 이 컨테이너를, `readText` 가 **원소**를 읽는다.
 *
 * `join` 만으로는 모자란다 — 배열 안에 객체가 있어도 `join` 은 던지지 않고
 * `"[object Object]"` 를 만들어 **조용히 틀린 도메인**을 보여주고, 그러고도
 * `primary`(= `domains[0]`)는 그대로 React 자식이 되어 렌더에서 터진다.
 */
export function toDomains(domains: readonly string[]): DomainsVM {
  const list = domains.map((d, i) => readText(d, `domains[${i}]`));
  return {
    primary: list[0] ?? null,
    all: list.join(", "),
    extra: Math.max(list.length - 1, 0),
  };
}

/** 발급 진행 로그 한 줄 — **시각이 이미 화면 표기로 바뀐 상태**다. */
export interface IssueLogLineVM {
  at: string;
  level: "info" | "error";
  msg: string;
}

/**
 * 발급 로그 → 화면 줄. 시각 포맷팅이 **패널이 아니라 여기**로 온 이유는 하나다:
 * `at.replace("T", " ")` 가 관문 밖(렌더 도중)에서 불리면 §9 카드가 아니라 일반
 * 오류 화면이 된다(`LoadNodePageAllArgs.toViewModel` 문서의 실측). 변환이 곧
 * 읽기가 되게 옮겼다 — 패널은 이제 문자열을 찍기만 한다.
 */
export function toIssueLogLines(entries: readonly IssueLogEntry[]): IssueLogLineVM[] {
  // `msg` 는 패널이 **그대로 React 자식으로** 그린다 — `at` 처럼 저절로 던지는
  // 호출이 없으니 여기서 타입을 본다. `level` 은 `=== "error"` 비교에만 쓰이고
  // (className 이 갈릴 뿐) 화면에 안 나가므로 읽지 않는다.
  return entries.map((e, i) => ({
    at: e.at.replace("T", " ").slice(0, 19),
    level: e.level,
    msg: readText(e.msg, `issueLog[${i}].msg`),
  }));
}

/**
 * TXT 안내 → 화면 모양. 패널이 **세 문자열을 전부 React 자식으로** 찍으므로
 * (`dns-challenge-panel.tsx` 의 표), `map`(컨테이너)에 더해 셋 다 원소 깊이로
 * 읽는다. 속성 접근만으로는 「`recordValue` 가 객체」를 못 잡는다 —
 * **Task 7 이 이 표를 실제로 그린다.**
 */
export function toDnsRecords(records: readonly DnsChallengeRecordView[]): DnsChallengeRecordView[] {
  return records.map((r, i) => ({
    domain: readText(r.domain, `dnsRecords[${i}].domain`),
    recordName: readText(r.recordName, `dnsRecords[${i}].recordName`),
    recordValue: readText(r.recordValue, `dnsRecords[${i}].recordValue`),
  }));
}

/**
 * 노드 응답의 ISO 문자열 → `Date`. **`toViewModel` 안에서 부른다.**
 *
 * `new Date("아무거나")` 는 던지지 않고 Invalid Date 를 낸다 — 그대로 넘기면
 * 화면이 `toISOString()` 을 부르는 **렌더 도중에** RangeError 가 나고, 하드
 * 로드에서 그건 §9 카드가 아니라 일반 500 화면이 된다. 그래서 여기서 같은
 * 호출을 한 번 해 둔다.
 *
 * `null` 은 계약이 정한 값이라 그대로 통과시킨다. **필드가 통째로 빠진
 * 응답은 통과하지 않는다** — `=== null` 이 아니므로 `new Date(undefined)` 가
 * Invalid Date 가 되어 바로 아래에서 던진다.
 */
export function readInstant(iso: string | null): Date | null {
  if (iso === null) return null;
  const at = new Date(iso);
  at.toISOString(); // ← `fmtInstant` 가 부르는 그 호출. Invalid Date 면 여기서 던진다.
  return at;
}

/**
 * 노드 응답 → `DnsOrderVM`. **로컬 상세와 같은 값으로 가른다** —
 * `dnsSubmittedAt`(= 노드 행의 `dnsOrder.submittedAt`)이 있으면 사용자가 이미
 * 「계속」을 눌러 CA 검증이 도는 중이다.
 *
 * **`status` 로 가르지 않는 것이 요점이다.** 최초 발급이라면 `status` 로도
 * 갈리지만(`awaiting-dns` / `issuing`) **갱신은 두 단계 모두 `issued` 를
 * 유지한다** — 그때 `status` 를 보면 단계를 알 수 없고, 잘못 고르면 사용자가
 * 넣어야 할 TXT 를 감추거나 이미 검증 중인 주문에 TXT 를 넣으라고 말한다
 * (`DnsOrderPhase` 문서).
 */
export function toNodeDnsOrder(
  records: readonly DnsChallengeRecordView[] | null,
  submittedAt: string | null,
): DnsOrderVM | null {
  if (records === null) return null;
  // **`verifying` 에는 안내를 싣지 않는다** — 그 단계에서는 패널이 뜨지 않아
  // 아무도 그리지 않는다(`DnsOrderVM` 문서). 읽지 않을 값을 읽으면 그리지도
  // 않을 응답 조각 때문에 화면이 카드로 바뀐다.
  if (submittedAt) return { phase: "verifying" };
  return { phase: "awaiting-txt", records: toDnsRecords(records) };
}

/** 인증서 하나가 연결한 서버 한 줄(상세의 「연결된 서버」 표). */
export interface LinkedServerVM {
  id: string;
  serverName: string;
  listen: number;
}

/**
 * 서버 한 행 → 「연결된 서버」 한 줄. **로컬·노드가 같은 함수를 쓴다** — 로컬은
 * 타입이 실재하는 컬럼이라 읽기가 무해한 무동작이고, 노드는 검증되지 않은
 * 응답이라 여기가 유일한 관문이다.
 *
 * `serverName`·`listen` 둘 다 표의 셀에 **그대로** 들어간다. `id` 는 `key` 와
 * href 문자열에만 쓰여(둘 다 문자열화된다) 이 부류가 아니다.
 */
export function toLinkedServer(row: { id: string; serverName: string; listen: number }): LinkedServerVM {
  return {
    id: row.id,
    serverName: readText(row.serverName, "server.serverName"),
    listen: readNumber(row.listen, "server.listen"),
  };
}

/** 목록 한 줄. 로컬은 `certificates` 행에서, 노드는 `/api/v1/certificates` 항목에서 만든다. */
export interface CertificateRowVM {
  id: string;
  domains: DomainsVM;
  /** 사람이 붙인 이름. 없으면 `null` — 화면은 도메인만 그린다. */
  alias: string | null;
  status: CertificateStatus;
  source: CertificateSource;
  staging: boolean;
  expiresAt: Date | null;
  /** 이 인증서를 TLS 로 쓰는 서버 수. **같은 출처의 서버 목록**에서 세어야 한다. */
  linkedCount: number;
}

export interface CertificateListVM {
  items: CertificateRowVM[];
}

/**
 * 상세 화면이 쓰는 값 전부. 로컬은 `certificates` 행 + `nginx_servers` 조회,
 * 노드는 `/api/v1/certificates/:id` + `/api/v1/servers` 가 같은 모양을 만든다.
 */
export interface CertificateDetailVM {
  id: string;
  domains: DomainsVM;
  /** 사람이 붙인 이름. 있으면 제목이 되고 대표 도메인은 그 옆에 남는다. */
  alias: string | null;
  status: CertificateStatus;
  source: CertificateSource;
  staging: boolean;
  challengeType: CertificateChallengeType;
  /** `"cloudflare"` 면 TXT 를 provider API 가 대신 넣는다(문구가 달라진다). */
  dnsProvider: string | null;
  issuedAt: Date | null;
  expiresAt: Date | null;
  lastAttemptAt: Date | null;
  failureCount: number;
  lastError: string | null;
  issueLog: IssueLogLineVM[];
  dnsOrder: DnsOrderVM | null;
  linkedServers: LinkedServerVM[];
}

/**
 * 화면이 **자동 새로고침을 돌려야 하는 상태**인가. 폴러와 발급 로그의 「(실시간)」
 * 표시가 같은 판정을 써야 해서 함수로 뒀다(추출 전에는 같은 식이 두 줄에 적혀
 * 있었다 — `[id]/page.tsx:65,131`).
 *
 * **TXT 대기는 도는 쪽이 아니다.** 그 단계는 사람이 DNS 를 만지기 전에는 아무것도
 * 안 바뀌므로, 폴링하면 몇 분 동안 노드를 계속 부르면서 아무 변화도 못 본다.
 */
export function isLiveIssuance(cert: Pick<CertificateDetailVM, "status" | "dnsOrder">): boolean {
  return cert.status === "issuing" || cert.dnsOrder?.phase === "verifying";
}
