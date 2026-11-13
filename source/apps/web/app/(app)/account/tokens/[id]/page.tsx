import { readPageApi } from "@/lib/api-page";
import { tokenPresentation, type TokenApiRow } from "@/lib/token-api";
import { TokenDetail } from "../token-detail";

// Uses the authenticated Rust API view through the server-side page client.
export const runtime = "nodejs";

/**
 * 토큰 상세 + 해지.
 *
 * ⚠ **행 하나만 읽는 함수를 따로 쓰지 않고 `listTokens` 로 읽는다.** 서비스가
 * 내는 범위 좁히기(`kind` + `owner_user_id`)를 그대로 타기 위해서다 — id 로 바로
 * 읽는 경로를 새로 만들면 그 자리에서 범위를 다시 적어야 하고, 잊는 순간 id 를
 * 아는 사람이 남의 토큰 상세를 본다. 개인이 가질 토큰 수는 적어 비용도 문제가
 * 아니다.
 *
 * 없는 id 와 남의 id 는 **똑같이** `notFound()` 다 — 갈라 말하면 남의 토큰 id 의
 * 존재 여부를 알려주는 통로가 된다(`revokeToken` 의 비구별 원칙과 같은 결).
 */
export default async function AccountTokenDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const row = await readPageApi<TokenApiRow>(
    `/api/ui/v1/account/tokens/${encodeURIComponent(id)}`,
  );

  return (
    <>
      {/*
       * 시각·상태를 **여기서** 만들어 문자열로 내려보낸다. `TokenDetail` 은
       * `"use client"` 라 브라우저에서 다시 도는데, 거기서 포맷하면 목록(서버)과
       * 상세(브라우저)가 같은 토큰에 다른 시각을 보인다 — 실측으로 9시간 어긋났고
       * React #418(hydration text mismatch)이 났다(`tokens.ts` 주석 참조).
       */}
      <TokenDetail
        view={tokenPresentation(row)}
        basePath="/account/tokens"
        revoke={{ id: "account.tokens.revoke", args: [] }}
      />
    </>
  );
}
