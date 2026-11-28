import { readPageApi } from "@/lib/api-page";
import { tokenPresentation, type TokenApiRow } from "@/lib/token-api";
import { TokenDetail } from "@/app/(app)/account/tokens/token-detail";

// Uses the authenticated Rust API view through the server-side page client.
export const runtime = "nodejs";

/**
 * 서버 토큰 상세 + 해지.
 *
 * ⚠ **행 하나만 읽는 함수를 따로 쓰지 않고 `listTokens` 로 읽는다.** 서비스가 내는
 * 범위 좁히기(`kind = 'server'`)와 등급 게이트를 그대로 타기 위해서다 — id 로 바로
 * 읽는 경로를 새로 만들면 그 자리에서 범위를 다시 적어야 하고, 잊는 순간 id 를 아는
 * 사람이 서버 토큰 상세를 본다. 서버 토큰 수는 노드 수만큼이라 비용도 문제가 아니다.
 *
 * 없는 id 와 사용자 토큰의 id 는 **똑같이** `notFound()` 다 — 갈라 말하면 어느 id 가
 * 존재하는지 알려주는 통로가 된다(`revokeToken` 의 비구별 원칙과 같은 결).
 */
export default async function ServerTokenDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  // 게이트가 `listTokens` 앞이다 — 목록 페이지와 같은 이유(`page.tsx` 주석 참고).
  const row = await readPageApi<TokenApiRow>(
    `/api/ui/v1/admin/tokens/${encodeURIComponent(id)}`,
  );

  return (
    <>
      {/*
       * 해지 확인 창은 이 토큰과 연결 기록이 있는 게이트웨이의 접근도 끊긴다고
       * 안내한다. 업데이트 전 연결은 목록의 정보 아이콘과 /admin/security에서 안내한다.
       */}

      {/*
       * 시각·상태를 **여기서** 만들어 문자열로 내려보낸다. `TokenDetail` 은
       * `"use client"` 라 브라우저에서 다시 도는데, 거기서 포맷하면 목록(서버)과
       * 상세(브라우저)가 같은 토큰에 다른 시각을 보인다(실측: 9시간 차이 +
       * React #418 hydration mismatch).
       */}
      <TokenDetail
        view={tokenPresentation(row)}
        basePath="/admin/tokens"
        revoke={{ id: "admin.tokens.revoke", args: [] }}
      />
    </>
  );
}
