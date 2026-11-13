import { describe, expect, it } from "vitest";
import {
  daysLeft,
  fmtInstant,
  isLiveIssuance,
  readAlias,
  readInstant,
  readCertificateSource,
  readNullableText,
  readNumber,
  readText,
  statusLabel,
  toDnsRecords,
  toDomains,
  toIssueLogLines,
  toLinkedServer,
  toNodeDnsOrder,
  type CertificateDetailVM,
} from "./certificate-vm";

/**
 * 이 파일이 붙드는 것은 두 가지다.
 *
 * 1. **변환이 곧 관문이다.** 노드 응답은 검증되지 않은 캐스트라, 여기서 읽지
 *    않고 통과시킨 값은 렌더 도중에 던진다 — 그러면 §9 카드가 아니라 노드
 *    레이아웃 경계의 **일반 500 화면**이 뜬다(`LoadNodePageAllArgs.toViewModel`
 *    문서의 실측). 그래서 아래
 *    「어긋난 모양은 던진다」 검사들이 **던지는지**만 본다: 그 뒷단(던짐 →
 *    카드)은 `lib/node-page.test.tsx` 가 이미 붙들고 있으므로 사슬이 이어진다.
 *    방어적으로 대체값을 넣는 순간 이 검사들이 죽는다 — 그게 요점이다.
 *
 * 2. **`submittedAt` 이 계약에 없다는 사실.** 노드는 DNS-01 주문의 단계를
 *    `status` 로 가를 수 있는 만큼만 가르고, 갱신 주문에서는 **모른다고 말한다**.
 *    없는 값을 `undefined` 로 지어내면 화면이 「아직 안 눌렀다」를 단언하게 된다.
 */

describe("도메인 요약 — 목록·상세가 같은 규칙을 쓴다", () => {
  it("대표·전체·나머지 수를 함께 만든다", () => {
    expect(toDomains(["a.example.com", "b.example.com", "c.example.com"])).toEqual({
      primary: "a.example.com",
      all: "a.example.com, b.example.com, c.example.com",
      extra: 2,
    });
  });

  it("비어 있으면 대표가 null 이고 「외 N」이 안 붙는다", () => {
    expect(toDomains([])).toEqual({ primary: null, all: "", extra: 0 });
  });

  /**
   * 배열이 아니면 **여기서** 던져야 카드가 된다. 통과시키면 목록의 `[0]` 은
   * 문자열의 첫 글자를 그려서 조용히 잘못된 도메인을 보여준다.
   */
  it("배열이 아니면 던진다 — 조용히 첫 글자를 그리지 않는다", () => {
    expect(() => toDomains("a.example.com" as unknown as string[])).toThrow();
    expect(() => toDomains(null as unknown as string[])).toThrow();
  });

  /**
   * **컨테이너 깊이에서 멈추면 절반만 닫힌다.** `join` 은 원소를 검사하지 않고
   * `"[object Object]"` 를 만들어 조용히 틀린 도메인을 보여주고, 그러고도
   * `primary` 는 그대로 React 자식이 되어 렌더에서 터진다.
   */
  it("원소가 문자열이 아니면 던진다 — join 은 그걸 못 잡는다", () => {
    expect(() => toDomains([{ host: "a.example.com" }] as unknown as string[])).toThrow();
    expect(() => toDomains(["ok.example.com", null] as unknown as string[])).toThrow();
  });
});

describe("발급 로그 — 시각 포맷팅이 관문 안으로 왔다", () => {
  it("ISO 시각을 화면 표기로 바꾼다", () => {
    expect(toIssueLogLines([{ at: "2026-08-29T12:34:56.789Z", level: "info", msg: "시작" }])).toEqual([
      { at: "2026-08-29 12:34:56", level: "info", msg: "시작" },
    ]);
  });

  /**
   * 패널이 `at.replace(…)` 를 부르던 것을 여기로 옮긴 이유가 이것이다 — 그
   * 호출이 렌더 도중에 있으면 어긋난 로그가 화면 전체를 오류로 만든다.
   */
  it("배열이 아니거나 at 이 문자열이 아니면 던진다", () => {
    expect(() => toIssueLogLines({} as never)).toThrow();
    expect(() => toIssueLogLines([{ at: 1234, level: "info", msg: "x" }] as never)).toThrow();
  });

  /**
   * `at` 은 `replace` 가 저절로 잡지만 **`msg` 는 옮겨 담기뿐**이라 통과한다 —
   * 그리고 패널이 그것을 React 자식으로 그린다.
   */
  it("msg 가 문자열이 아니면 던진다 — at 만 읽으면 안 잡힌다", () => {
    expect(() =>
      toIssueLogLines([{ at: "2026-08-29T00:00:00.000Z", level: "info", msg: { text: "x" } }] as never),
    ).toThrow();
  });
});

describe("TXT 안내 — 패널이 읽는 깊이로 여기서 읽는다", () => {
  it("세 필드만 옮긴다", () => {
    expect(
      toDnsRecords([
        { domain: "a.example.com", recordName: "_acme-challenge.a.example.com", recordValue: "abc" },
      ]),
    ).toEqual([{ domain: "a.example.com", recordName: "_acme-challenge.a.example.com", recordValue: "abc" }]);
  });

  it("배열이 아니면 던진다", () => {
    expect(() => toDnsRecords("nope" as never)).toThrow();
  });

  /**
   * 속성 접근만으로는 「값이 객체」를 못 잡는다. **Task 7 이 이 셋을 표에
   * 그린다** — 그때가 되어서야 터지면 관문 밖이다.
   */
  it("세 문자열 중 하나라도 문자열이 아니면 던진다", () => {
    const ok = { domain: "a.example.com", recordName: "_acme-challenge.a.example.com", recordValue: "v" };
    expect(() => toDnsRecords([{ ...ok, recordValue: { v: 1 } }] as never)).toThrow();
    expect(() => toDnsRecords([{ ...ok, recordName: 42 }] as never)).toThrow();
    expect(() => toDnsRecords([{ ...ok, domain: null }] as never)).toThrow();
  });
});

describe("연결된 서버 — 표의 두 셀이 React 자식이다", () => {
  it("행을 그대로 옮긴다", () => {
    expect(toLinkedServer({ id: "srv-1", serverName: "a.example.com", listen: 80 })).toEqual({
      id: "srv-1",
      serverName: "a.example.com",
      listen: 80,
    });
  });

  it("serverName 이 문자열이 아니거나 listen 이 숫자가 아니면 던진다", () => {
    expect(() => toLinkedServer({ id: "srv-1", serverName: { v: 1 }, listen: 80 } as never)).toThrow();
    expect(() => toLinkedServer({ id: "srv-1", serverName: "a.example.com", listen: "80" } as never)).toThrow();
  });
});

describe("원소 깊이 읽기 — 저절로 던지는 호출이 없는 자리", () => {
  it("문자열·숫자는 그대로 통과한다", () => {
    expect(readText("x", "f")).toBe("x");
    expect(readNumber(3, "f")).toBe(3);
    expect(readNullableText(null, "f")).toBeNull();
    expect(readNullableText("x", "f")).toBe("x");
  });

  it("객체·배열·undefined 는 던진다 — React 자식으로 못 가는 것들이다", () => {
    expect(() => readText({} as never, "f")).toThrow();
    expect(() => readText([] as never, "f")).toThrow();
    expect(() => readText(undefined as never, "f")).toThrow();
    expect(() => readNumber("3" as never, "f")).toThrow();
    expect(() => readNullableText({} as never, "f")).toThrow();
  });

  /** `NaN` 은 「NaN」으로 그려질 뿐 렌더를 깨뜨리지 않는다 — 이 함수가 막을 것이 아니다. */
  it("NaN 은 막지 않는다", () => {
    expect(readNumber(Number.NaN, "f")).toBeNaN();
  });

  /**
   * 별칭은 `readNullableText` 와 **한 가지가 다르다** — `undefined` 도 통과한다.
   * 별칭을 모르는 구 노드의 응답에는 키 자체가 없고, 그것은 「없음」이지 오류가 아니다.
   */
  it("별칭은 문자열·null·undefined(구 노드)를 통과시키고 그 밖은 던진다", () => {
    expect(readAlias("운영 API", "alias")).toBe("운영 API");
    expect(readAlias(null, "alias")).toBeNull();
    expect(readAlias(undefined, "alias")).toBeNull();
    expect(() => readAlias({} as never, "alias")).toThrow();
    expect(() => readAlias(3 as never, "alias")).toThrow();
  });
});

/**
 * `source` 는 **화면에 그려지지 않는다.** 그래서 위 부류와 다르고, 그런데도
 * 관문에서 읽어야 하는 이유가 따로 있다 — 수정 화면이 `source !== "acme"` 로
 * **어느 화면을 보여줄지**를 정한다(`[cid]/edit/page.tsx`). 통과시키면 노드가
 * 이상한 값을 줬을 때 §9 카드가 떠야 할 자리에서 **말없이 교체 화면**이 뜬다.
 */
describe("source — 렌더되지 않지만 분기를 고르는 값", () => {
  it("계약이 정한 둘은 그대로 통과한다", () => {
    expect(readCertificateSource("acme", "source")).toBe("acme");
    expect(readCertificateSource("manual", "source")).toBe("manual");
  });

  it("그 밖의 값은 던진다 — 조용히 다른 화면으로 보내지 않는다", () => {
    // `!== "acme"` 는 이것들 전부에 대해 참이라, 읽지 않으면 전부 교체 화면으로 간다.
    expect(() => readCertificateSource("ACME" as never, "source")).toThrow();
    expect(() => readCertificateSource("" as never, "source")).toThrow();
    expect(() => readCertificateSource(null as never, "source")).toThrow();
    expect(() => readCertificateSource(undefined as never, "source")).toThrow();
    expect(() => readCertificateSource({} as never, "source")).toThrow();
  });
});

describe("시각 — 노드는 ISO 문자열을 준다", () => {
  it("null 은 그대로 통과한다 — 계약이 정한 값이다", () => {
    expect(readInstant(null)).toBeNull();
  });

  it("ISO 문자열을 Date 로 되돌린다", () => {
    expect(readInstant("2026-08-29T12:00:00.000Z")?.toISOString()).toBe("2026-08-29T12:00:00.000Z");
  });

  /**
   * `new Date("아무거나")` 는 던지지 않고 Invalid Date 를 낸다. 그대로 넘기면
   * 화면의 `fmtInstant` 가 `toISOString()` 에서 던지고, 그 자리는 관문 **밖**이다.
   */
  it("말이 안 되는 값과 아예 빠진 필드는 던진다 — Invalid Date 를 실어 나르지 않는다", () => {
    expect(() => readInstant("어제")).toThrow();
    expect(() => readInstant(undefined as unknown as string | null)).toThrow();
  });
});

describe("DNS-01 주문 단계 — submittedAt 하나로 가른다", () => {
  const RECORDS = [{ domain: "a.example.com", recordName: "_acme-challenge.a.example.com", recordValue: "v" }];
  const SUBMITTED = "2026-08-29T00:00:00.000Z";

  it("주문이 없으면 null 이다", () => {
    expect(toNodeDnsOrder(null, null)).toBeNull();
    expect(toNodeDnsOrder(null, SUBMITTED)).toBeNull();
  });

  it("아직 안 눌렀으면 TXT 대기다", () => {
    expect(toNodeDnsOrder(RECORDS, null)?.phase).toBe("awaiting-txt");
  });

  it("눌렀으면 검증 중이다", () => {
    expect(toNodeDnsOrder(RECORDS, SUBMITTED)?.phase).toBe("verifying");
  });

  /**
   * **`status` 를 보고 있었다면 갱신에서 틀린다.** 갱신 주문은 두 단계 모두
   * `issued` 를 유지하므로(서빙 중인 인증서를 지키려는 의도적 설계), 상태만
   * 보면 「TXT 를 넣으세요」와 「검증 중단」 중 어느 쪽인지 알 수 없다. 이
   * 검사는 **같은 `status` 에서 두 단계가 모두 나온다**는 것을 붙든다 —
   * `toNodeDnsOrder` 가 다시 `status` 를 보게 되면 둘 중 하나가 죽는다.
   */
  it("같은 status(갱신)에서도 두 단계가 갈린다", () => {
    expect(toNodeDnsOrder(RECORDS, null)?.phase).toBe("awaiting-txt");
    expect(toNodeDnsOrder(RECORDS, SUBMITTED)?.phase).toBe("verifying");
  });

  /**
  * **TXT 안내는 그것을 그리는 단계에만 실린다.** 검증 중에는 패널이 안 뜨므로
  * 그때 읽으면 아무도 안 그릴 응답 조각 때문에 화면이 카드로 바뀐다.
  */
  it("그리는 단계에만 TXT 안내를 싣는다", () => {
    expect(toNodeDnsOrder(RECORDS, null)).toEqual({ phase: "awaiting-txt", records: RECORDS });
    // 검증 중에는 `records` 키 자체가 없다 — 타입이 그 자리를 막고 있다.
    expect(toNodeDnsOrder(RECORDS, SUBMITTED)).toEqual({ phase: "verifying" });
  });

  /**
   * 검증 단계에서 `records` 를 **읽지 않는다**는 것을 값이 아니라 거동으로
   * 붙든다: 어긋난 안내를 줘도 그 단계에서는 던지지 않아야 한다(그리지 않으니까).
   */
  it("검증 중에는 어긋난 TXT 안내로도 던지지 않는다 — 읽지 않기 때문이다", () => {
    expect(() => toNodeDnsOrder("망가짐" as never, SUBMITTED)).not.toThrow();
    expect(() => toNodeDnsOrder("망가짐" as never, null)).toThrow();
  });
});

describe("자동 새로고침 판정", () => {
  function cert(over: Partial<Pick<CertificateDetailVM, "status" | "dnsOrder">>) {
    return { status: "issued" as const, dnsOrder: null, ...over };
  }

  it("발급 중이면 돈다", () => {
    expect(isLiveIssuance(cert({ status: "issuing" }))).toBe(true);
  });

  it("TXT 대기 중에는 돌지 않는다 — 사용자가 움직이기 전에는 아무것도 안 바뀐다", () => {
    expect(isLiveIssuance(cert({ dnsOrder: { phase: "awaiting-txt", records: [] } }))).toBe(false);
  });

  it("검증 중이면 돈다", () => {
    expect(isLiveIssuance(cert({ dnsOrder: { phase: "verifying" } }))).toBe(true);
  });

  it("주문도 없고 발급 중도 아니면 돌지 않는다", () => {
    expect(isLiveIssuance(cert({}))).toBe(false);
  });
});

describe("표기 헬퍼 — 목록과 상세가 한 벌을 쓴다", () => {
  it("상태 문구는 한 곳에서 온다", () => {
    expect(statusLabel("awaiting-dns")).toBe("TXT 대기");
    // 모르는 값은 그대로 보여준다 — 추출 전 두 화면이 하던 그대로다.
    expect(statusLabel("bizarre")).toBe("bizarre");
  });

  it("만료일이 없으면 남은 일수도 없다", () => {
    expect(daysLeft(null)).toBeNull();
  });

  it("지난 만료일은 음수다 — 목록이 「만료」 배지를 그 부호로 고른다", () => {
    expect(daysLeft(new Date(Date.now() - 3 * 24 * 60 * 60 * 1000))).toBeLessThan(0);
  });

  it("시각 표기는 T 를 공백으로 바꾸고 초까지만 남긴다", () => {
    expect(fmtInstant(new Date("2026-08-29T12:34:56.789Z"))).toBe("2026-08-29 12:34:56");
    expect(fmtInstant(null)).toBe("—");
  });
});
