import { FormStack, Banner } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { Icon, linkClassName, PageHeader } from "@repo/ui";
import { readPageApi } from "@/lib/api-page";
import type { TokenApiList } from "@/lib/token-api";
import Link from "next/link";
import { TokenForm } from "../token-form";

// Uses the authenticated Rust API view through the server-side page client.
export const runtime = "nodejs";

/**
 * 토큰 발급 페이지.
 *
 * ⚠ **체크박스 목록은 `issuableScopes("user", 내 등급)` 이다.**
 * - `node:pair` 는 사용자 토큰 어휘에 아예 없다 — admin 이 열어도 **보이지 않는다**
 *   (비활성화가 아니라 부재다, 스펙 §4).
 * - 내 등급이 못 하는 일은 보여주지 않는다 — 보여주고 나중에 거절하면 사용자는
 *   왜 안 되는지 알 수 없다(스펙 §8-1). viewer 에게 `config:write` 가 없는 이유다.
 *
 * `ALL_SCOPES` 나 `scopesForKind` 로 바꾸면 둘 다 깨진다. 서비스가 다시 막기는
 * 하지만(`validateScopes`), 그때는 **고를 수 없는 것을 고르게 한 뒤** 거절하는
 * 화면이 된다 — 방어를 한 겹으로 줄이지 마라.
 */
export default async function NewAccountTokenPage() {
  const { issuableScopes: scopes } = await readPageApi<TokenApiList>(
    "/api/ui/v1/account/tokens",
  );

  return (
    <section className={layout.page}>
      <PageHeader
        title={<>새 액세스 토큰</>}
        description={
          <>
            발급이 끝나면 토큰 값을 <strong>한 번만</strong> 보여줍니다.
          </>
        }

        eyebrow={
          <Link
            href="/account/tokens"
            className={linkClassName({ variant: "back" })}
          >
            <Icon name="arrowLeft" size={14} />
            토큰 목록으로
          </Link>
        }
      />

      <FormStack>
        <Banner
          tone="info"
          title={<>고른 권한만 이 토큰으로 할 수 있습니다.</>}
        />
        <TokenForm
          kind="user"
          scopes={scopes}
          action={{ id: "account.tokens.issue", args: [] }}
          basePath="/account/tokens"
        />
      </FormStack>
    </section>
  );
}
