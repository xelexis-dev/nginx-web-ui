import { FormStack, Banner } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { Icon, linkClassName, PageHeader } from "@repo/ui";
import { readPageApi } from "@/lib/api-page";
import type { TokenApiList } from "@/lib/token-api";
import Link from "next/link";
import { TokenForm } from "@/app/(app)/account/tokens/token-form";

// Uses the authenticated Rust API view through the server-side page client.
export const runtime = "nodejs";

/**
 * 서버 토큰 발급.
 *
 * ⚠ **방향**: 여기서 만든 토큰은 **다른 게이트웨이가 이 서버에 제시하는** 자격증명이다
 * (`page.tsx` 헤더 참고). 이 서버가 남을 노드로 붙일 때 쓰는 토큰은 **그 기계의**
 * 같은 화면에서 나온다.
 *
 * ⚠ **체크박스 목록은 `issuableScopes("server", …)` 다 — 그리고 그 함수는 서버
 * 토큰에서 `role` 을 보지 않는다.** 의도된 것이다: `node:pair` 는 발급한 **사람**이
 * 아니라 **인스턴스**에 속하는 권한이라, 사람의 등급이 섞이면 그 사람이 강등되는
 * 순간 서버 간 연결의 의미가 흔들린다.
 *
 * 그래서 「누가 발급할 수 있나」는 이 목록이 아니라 **게이트**가 답한다 — 화면의
 * `requireRole("user:manage")` 와 서비스의 `role !== "admin"` 두 겹이다.
 * `ALL_SCOPES` 를 직접 넘기지 마라: 그러면 종류별 상한이 어디에서 정해지는지가
 * 사라지고, 사용자 토큰 화면과의 차이가 우연이 된다.
 */
export default async function NewServerTokenPage() {
  // 게이트가 먼저다 — 아래 `issuableScopes` 는 `node:pair` 를 그리므로, 게이트 없이
  // 그리면 일반 사용자에게 「그런 권한이 있다」는 사실부터 알려주게 된다(스펙 §4).
  const { issuableScopes: scopes } = await readPageApi<TokenApiList>(
    "/api/ui/v1/admin/tokens",
  );

  return (
    <section className={layout.page}>
      <PageHeader
        title={<>새 서버 토큰</>}
        description={
          <>
            발급이 끝나면 토큰 값을 <strong>한 번만</strong> 보여줍니다.
          </>
        }

        eyebrow={
          <Link
            href="/admin/tokens"
            className={linkClassName({ variant: "back" })}
          >
            <Icon name="arrowLeft" size={14} />
            서버 토큰 목록으로
          </Link>
        }
      />

      {/*
       * `kind="server"` 는 **여기 코드**가 정한다. 폼은 그 값을 제출하지 않는다 —
       * 실제 종류는 서버 액션이 다시 코드로 쓴다(`actions.ts`). 이 prop 이 바꾸는
       * 것은 이름 예시뿐이고, 그게 맞다.
       */}
      <FormStack>
        <Banner
          tone="info"
          title={
            <>
              다른 게이트웨이가 이 서버를 노드로 연결할 때 제시하는 토큰입니다.
              이름에 어느 게이트웨이에 줄 것인지 적어 두세요.
            </>
          }
        />
        <TokenForm
          kind="server"
          scopes={scopes}
          action={{ id: "admin.tokens.issue", args: [] }}
          basePath="/admin/tokens"
        />
      </FormStack>
    </section>
  );
}
