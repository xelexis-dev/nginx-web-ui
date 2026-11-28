import { readPageApi } from "@/lib/api-page";
import { readRevokedTokenName, tokenPresentation, type TokenApiList } from "@/lib/token-api";
import { ServerTokensView } from "@/app/(app)/account/tokens/token-list";

// Uses the authenticated Rust API view through the server-side page client.
export const runtime = "nodejs";

/**
 * 서버 토큰 목록 — **이 서버가 발급한** 토큰들이다.
 *
 * ── 방향을 먼저 못박는다 ────────────────────────────────────────────────
 *
 * ⚠ **여기서 발급한 토큰으로 이 서버가 노드를 붙이는 것이 아니다. 반대다.**
 * `POST /api/v1/pair` 는 **이 인스턴스 위의** 엔드포인트이고, 그것을 부르는 쪽이
 * 게이트웨이다(`app/api/v1/pair/route.ts` 가 `gatewayTokens` 행을 만든다 =
 * 「누가 나에게 붙었나」). 이 서버가 다른 기계를 노드로 붙일 때 쓰는 토큰은
 * **그 기계의** `/admin/tokens` 에서 나온다(스펙 §8-4: 「연결할 **노드의** 관리자
 * 패널 → 액세스 토큰에서 발급한 토큰을 붙여넣으세요」).
 *
 * 그래서 이 화면의 문구는 **다른 게이트웨이가 이 서버를 자기 노드로 연결한다**로
 * 적는다. 거꾸로 적으면 노드를 추가하려던 관리자가 아무 효과도 없는 토큰을 만들고,
 * 그 값은 하필 **자기 서버의 제어권을 넘기는 자격증명**이다.
 *
 * ── 왜 `/admin/security` 에 합치지 않는가 ────────────────────────────────
 *
 * 반대 방향이라서가 아니다 — 둘은 **같은 관계의 두 단계**다. 여기서 내준 열쇠가
 * 상대가 쓰고 나면 저기 「연결된 게이트웨이」의 한 줄이 된다.
 *
 * 서버 토큰 해지는 이 토큰과 연결 기록이 있는 게이트웨이의 접근도 함께 해지한다.
 * 업데이트 전에 연결된 게이트웨이는 이 토큰과의 연결 정보가 없어 함께 해지되지
 * 않으므로(`0032_auth_compatibility.sql` 은 `gateway_tokens.access_token_id` 를 채우지
 * 않는다) 「연결된 게이트웨이」에서 해지한다. 연결별 해지도 거기서 할 수 있다.
 *
 * ── 왜 목록도 admin 전용인가 ────────────────────────────────────────────
 *
 * 스펙 §2 가 「일반 사용자는 서버 토큰의 존재도 모른다」고 못박는다. 목록만으로도
 * **어느 게이트웨이가 이 서버를 관리하고 있고** 언제 마지막으로 썼는지가 드러난다 —
 * 이름에 상대의 정체를 적게 되어 있기 때문이다. 읽기라고 가벼운 것이 아니다.
 */
export default async function ServerTokensPage({
  searchParams,
}: {
  searchParams: Promise<{ revoked?: string | string[] }>;
}) {
  /**
   * ⚠ **게이트가 `listTokens` 보다 앞이다.** `requireServerTokenActor()` 의 첫 줄이
   * `requireRole("user:manage")` 이고, 서버 범위(`{ kind: "server" }`)는 그것을
   * 지나야만 만들어진다.
   *
   * 순서를 뒤집거나 게이트를 빼면 화면이 **막히는 게 아니라 500 이 난다** —
   * `listTokens` 는 비-admin 의 서버 범위 호출에 `AccessTokenError` 를 **던진다**.
   * 사용자에게 「권한이 없습니다」와 「서버 오류」는 전혀 다른 말이다.
   *
   * ⚠ **다만 이 게이트와 서비스의 게이트는 같은 술어가 아니다.** 여기는
   * `can(role, "user:manage")` 이고 서비스는 `role !== "admin"` 이다. 오늘은
   * `rbac.ts` 의 MATRIX 가 `user:manage` 를 admin 에게만 주어 둘이 일치하지만,
   * **표가 비-admin 에게 `user:manage` 를 주는 날 화면은 통과시키고 `listTokens` 가
   * 던진다.** 그때 이 페이지를 받아 줄 것은 루트 `app/error.tsx` 뿐이라, 결과는
   * 위가 피하려던 바로 그 「깨지는 것」이다.
   *
   * 여기에 추측성 처리를 더하지 않는다 — 오늘 그 경로는 없고, 있지도 않은 경우를
   * 위한 분기는 검사할 수 없다. 대신 **갈라지는 날 무엇을 해야 하는지**를 적어 둔다:
   * 세 페이지의 게이트를 `role !== "admin"` 으로 맞추거나, 액션이 하는 것처럼
   * (`actions.ts` 의 `issueErrorState`) `AccessTokenError` 를 잡아 화면에 그린다.
   */
  const { tokens } = await readPageApi<TokenApiList>("/api/ui/v1/admin/tokens");
  const revokedName = await readRevokedTokenName("admin", (await searchParams).revoked);
  return <ServerTokensView rows={tokens.map(tokenPresentation)} revokedName={revokedName} />;
}
