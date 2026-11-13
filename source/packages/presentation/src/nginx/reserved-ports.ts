/**
 * 「이 포트로 리스너를 하나 더 열면 인스턴스가 죽는가」 판정.
 *
 * ## 기준은 하나다 — 같은 (프로토콜, 포트)에 리스닝 소켓이 이미 있는가
 *
 * 이 제품은 **한 컨테이너 안에 nginx · node · postgres 를 함께** 돌린다. 커널
 * 규칙은 단순하다: **같은 프로토콜·같은 주소·같은 포트**에 두 번째 `bind()` 를
 * 하면 `EADDRINUSE` 다. nginx 마스터는 http 블록과 stream 블록의 리스닝 소켓을
 * **한 번에** 연다. 둘이 겹치면 두 번째 bind 가 실패하고 nginx 는 **아예 뜨지
 * 않는다** — 관리 UI 까지 함께 사라진다.
 *
 * 실측(nginx 1.30.4, 이미지의 그 바이너리):
 *
 * | 설정 | `nginx -t` | 기동 |
 * |---|---|---|
 * | http :8443 만 / stream tcp :8443 만 | 통과 | 각각 뜬다 |
 * | **http :8443 + stream tcp :8443** | **통과** | **실패** — `bind() to 0.0.0.0:8443 failed (98: …)` |
 * | http :8080/:80/:443 여러 개 | 통과 | 뜬다 — **정상 가상호스팅**(리스닝 소켓 하나를 공유) |
 * | stream tcp :15432 두 개 | 통과 | 뜬다 (소켓 공유) |
 * | stream tcp :15432 + stream udp :15432 | 통과 | 뜬다 |
 * | stream udp :8080 + http :8080 | 통과 | 뜬다 |
 * | **다른 프로세스**가 tcp :15999 를 쥔 상태에서 stream **udp** :15999 | — | **뜬다** |
 * | 〃 stream **tcp** :15999 (대조군) | — | **실패** — `EADDRINUSE` |
 *
 * 여기서 세 가지가 나온다.
 *
 * 1. **포트가 `nginx.conf` 에 적혀 있는지는 상관없다.** 중요한 것은 *지금 듣고
 *    있는가* 다. 저장된 리버스 프록시가 :8443 을 듣게 만들면, `nginx.conf` 는
 *    :8443 을 한 번도 언급하지 않아도 그 포트는 **점유된 것**이다. 그래서 예약
 *    집합은 `nginx.conf` 뿐 아니라 **DB 에 저장된 행에서도** 나와야 한다.
 * 2. **방향이 대칭이다.** L7 먼저 + L4 나중이든 그 반대든 결과는 같다. 그래서
 *    `http` 맥락에서는 stream 이 쥔 포트를, `stream` 맥락에서는 http 가 쥔 포트를
 *    거절한다.
 * 3. **UDP 는 TCP 예약 포트와 겹치지 않는다.** 이름 확인 서비스처럼 UDP도
 *    여는 구성요소의 포트만 별도 `processUdp` 집합으로 받아 검사한다.
 *
 * 같은 프로토콜·같은 포트의 **동종** 리스너는 겹치지 않는다(http 끼리, stream
 * 끼리 소켓을 공유한다 — 위 표). 그래서 자기 테이블은 예약 대상이 아니고,
 * 그 덕에 **행을 수정할 때 자기 자신을 제외할 필요가 없다.**
 *
 * ## 왜 `nginx -t` 가 못 막는가
 *
 * `-t` 는 **바인드를 하지 않는다** — 위 표에서 치명적인 두 줄이 모두 통과했다.
 * 적용 경로의 `reload()` 도 `nginx -s reload` 의 **종료 코드만** 보는데, 그
 * 명령은 마스터에 신호만 주고 끝나고 실제 `bind()` 실패는 비동기로 에러 로그에만
 * 남는다. 그래서 **적용은 초록 성공을 보고하고** 고장은 다음 재시작에 드러난다.
 * 검사를 **입력 시점에** 두는 것 말고는 막을 자리가 없다.
 *
 * ## 왜 목록을 여기에 박지 않는가
 *
 * 목록의 출처가 셋이고 셋 다 런타임 값이다: `PORT`(env), `DATABASE_URL`(env),
 * 그리고 **DB 에 저장된 서버·stream 행**. 이 패키지는 순수해야 하므로(렌더·검증만
 * 한다) env 도 DB 도 읽지 않고 **집합을 인자로 받는다.** 실제 값을 만드는 곳은
 * `apps/web/lib/reserved-ports.ts` 하나뿐이다.
 */

import { ADMIN_UI_PORT } from "./self-proxy";

/**
 * `listen` 이 만들 리스닝 소켓의 종류. 포트가 같아도 이 값에 따라 판정이 갈린다 —
 * 프로토콜이 다르면 소켓도 다르고, 겹치지 않는다.
 */
export type ListenContext = "http" | "stream-tcp" | "stream-udp";

export interface ReservedPorts {
  /**
   * **이 인스턴스의 다른 프로세스**가 TCP 로 듣는 포트(node 앱, 내장 DB). nginx 가
   * 먼저 잡으면 그 프로세스가 뜨지 못하고, `web` 이 안 뜨면 관리 UI 도 없다.
   * TCP 리스너라면 http 든 stream 이든 치명적이다.
   */
  readonly process: readonly number[];
  /** UDP로도 듣는 내부 구성요소의 포트. 누락은 빈 집합이다. */
  readonly processUdp?: readonly number[];
  /**
   * 이 nginx 의 **http 블록**이 듣는(듣게 될) 포트 — `docker/nginx.conf` 의 기본
   * 리스너 + **DB 에 저장된 L7 서버 행**. 전부 TCP 다. 여기에 `stream` TCP 리스너를
   * 얹으면 마스터가 `bind()` 에 실패해 nginx 가 시작되지 않는다. 반대로 http
   * server 를 하나 더 얹는 것은 정상 가상호스팅이라 **`http` 맥락에서는 쓰지 않는다.**
   */
  readonly http: readonly number[];
  /**
   * 이 nginx 의 **stream 블록**이 TCP 로 듣는(듣게 될) 포트 — DB 에 저장된 L4
   * stream 행 중 `tcp` 인 것. 여기에 http server 를 얹으면 같은 이유로 nginx 가
   * 시작되지 않는다. stream 끼리는 겹치지 않으므로 **`stream-tcp` 맥락에서는
   * 쓰지 않는다.**
   */
  readonly streamTcp: readonly number[];
  /**
   * 포트 → **그 포트를 차지한 것의 이름**(「리버스 프록시 shop.example.com」 등).
   *
   * **판정에는 쓰이지 않는다** — 오직 오류 문구를 구체적으로 만들기 위한 것이다.
   * 없으면 메시지가 「무엇이」를 말하지 못할 뿐 거절/통과 결과는 똑같다. 그래서
   * optional 이다: 판정에 필요한 세 집합은 필수, 표현용인 이것만 선택.
   *
   * 한 포트를 여럿이 차지할 수 있다(예: :443 에 리버스 프록시가 여러 개).
   * 그때는 **먼저 등록된 것 하나**를 담는다 — 목록을 다 나열하면 필드 아래
   * 한 줄에 들어가지 않고, 운영자가 찾아갈 곳 하나만 있으면 충분하다.
   */
  readonly owners?: Readonly<Record<number, string>>;
}

/** 예약 포트에 걸렸을 때 던지는 오류. 메시지가 그대로 사용자에게 보이는 문장이다. */
export class ReservedPortError extends Error {
  readonly port: number;
  readonly kind: "process" | "http" | "stream";

  constructor(port: number, kind: "process" | "http" | "stream", message: string) {
    super(message);
    this.name = "ReservedPortError";
    this.port = port;
    this.kind = kind;
  }
}

/**
 * 예약 포트면 `ReservedPortError` 를, 아니면 `null` 을 돌려준다. 던지지 않는 형태를
 * 따로 두는 것은 화면이 「이미 저장된 행이 예약 포트에 걸려 있다」를 표시할 수
 * 있게 하기 위해서다(`components/streams/stream-list.tsx`).
 */
export function findReservedPortConflict(
  port: number,
  context: ListenContext,
  reserved: ReservedPorts,
): ReservedPortError | null {
  // 「무엇이 차지했는가」. 없으면 그 자리를 통째로 비운다 — 「(알 수 없음)」 같은
  // 자리표시자는 아무것도 알려주지 않으면서 문장만 길게 만든다.
  const owner = reserved.owners?.[port];
  const by = owner ? `(${owner})` : "";

  if (context === "stream-udp") {
    if (!reserved.processUdp?.includes(port)) return null;
    return new ReservedPortError(
      port,
      "process",
      `UDP 포트 ${port} 는 이 인스턴스의 다른 구성요소${by}가 씁니다. 다른 포트를 쓰세요.`,
    );
  }

  if (reserved.process.includes(port)) {
    return new ReservedPortError(
      port,
      "process",
      `포트 ${port} 는 이 인스턴스의 다른 구성요소${by || "(웹 앱 또는 데이터베이스)"}가 씁니다. ` +
        `nginx 가 이 포트를 먼저 잡으면 그 구성요소가 시작하지 못해 관리 화면까지 함께 멈춥니다. 다른 포트를 쓰세요.`,
    );
  }

  if (context === "stream-tcp" && reserved.http.includes(port)) {
    // **「듣고 있습니다」라고 하지 않는다.** 예약 집합은 `enabled` 도 인증서 발급
    // 상태도 보지 않고 모으므로(그래야 「껐다가 다시 켜는」 경로로 구멍이 돌아오지
    // 않는다), 그중에는 **지금 리스닝하지 않는 것**이 섞여 있다. 그걸 「듣고 있다」고
    // 말하면 운영자가 목록에서 꺼진 행을 보고 제품이 거짓말한다고 판단하게 된다.
    return new ReservedPortError(
      port,
      "http",
      `포트 ${port} 는 이 nginx 의 HTTP 설정이 이미 차지하고 있습니다${by || "(관리 화면·HTTP·HTTPS 또는 등록된 리버스 프록시)"}. ` +
        `stream 이 같은 포트를 TCP 로 잡으면 nginx 가 아예 시작되지 않습니다 — 그 HTTP 항목이 지금 꺼져 있거나 ` +
        `인증서 발급 전이어도, 켜지는 순간 그렇게 됩니다. 다른 포트를 쓰거나 그 항목을 옮기세요.`,
    );
  }

  if (context === "http" && reserved.streamTcp.includes(port)) {
    return new ReservedPortError(
      port,
      "stream",
      `포트 ${port} 는 등록된 TCP stream 이 차지하고 있습니다${by}. ` +
        `같은 포트를 HTTP 쪽에서도 잡으면 nginx 가 아예 시작되지 않습니다 — 그 stream 이 지금 꺼져 있어도 ` +
        `켜지는 순간 그렇게 됩니다. 그 stream 을 옮기거나 지운 뒤 다시 시도하세요.`,
    );
  }

  return null;
}
