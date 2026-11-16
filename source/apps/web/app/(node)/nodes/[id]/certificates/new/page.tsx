import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { CertificateForm } from "@/components/certificates/certificate-form";
import { readPageApi } from "@/lib/api-page";
import type { SessionView } from "@repo/contracts";
import { authorizeNodePage } from "@/lib/node-page";

// 노드에 인증서를 새로 발급하는 화면. 폼 본문은
// components/certificates/certificate-form.tsx — 로컬 `(app)/certificates/new`
// 와 같은 컴포넌트다.
//
// **노드에서 읽어오는 것이 없다** — 이 폼은 후보 목록을 채우지 않는다(리버스
// 프록시 폼과 다른 점이다). 그래서 `loadNodePageAll` 이 아니라
// `authorizeNodePage` 로 접근만 연다: 부를 경로가 없는데 `loadNodePage*` 를 쓰면
// 실패 카드를 위해 아무 종점이나 골라 부르게 된다.
//
// ── 연락 이메일: 로컬 발급 화면과 **같은 1순위** ───────────────────────────
//
// 로컬 발급 화면(`(app)/certificates/new/page.tsx`)은 **지금 로그인한 사람**의
// 주소를 먼저 쓴다 — 이 화면도 같다. 발급 폼에 적은 주소는 실제로 그 대상의
// ACME 계정 연락처가 된다(`getOrCreateAccount` 는 이메일이 다르면 **갱신한다**),
// 그리고 그게 의도다: 지금 발급을 지시하는 사람이 이번 발급의 연락처다.
//
// 다른 것은 **폴백 하나**뿐이다. 로컬은 로그인 계정에 이메일이 없을 때(OAuth 전용
// 가입) 저장된 ACME 계정 주소로 물러나는데, 이 화면은 노드에서 아무것도 읽지
// 않으므로 빈 값으로 둔다 — 폼의 이메일 칸이 `required` 라 **빈 채로 제출되지
// 않는다**(조용히 덮이는 경로가 아니다).
//
// **수정 화면은 순서가 반대다**(`[cid]/edit/page.tsx`): 거기서는 노드에 저장된
// 주소가 1순위여야 무변경 저장이 무변경으로 남는다. 두 화면의 순서가 다른 것은
// 로컬과 같고, 의도다.
export const runtime = "nodejs";

export default async function NewNodeCertificatePage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const basePath = `/nodes/${id}/certificates`;

  const auth = await authorizeNodePage({
    nodeId: id,
    // 읽기가 아니라 쓰기 화면이다 — 그 등급이 없으면 폼을 띄우는 것 자체가
    // 헛일이다(로컬 `new/page.tsx` 의 `requireRole("config:write")` 에 대응).
    action: "config:write",
    retryHref: `${basePath}/new`,
    listHref: basePath,
  });
  if (!auth.ok) return (
    <section className={layout.page}>
      <PageHeader title="새 인증서" />
      {auth.render}
    </section>
  );

  // 위 `authorizeNodePage` 가 이미 로그인·등급을 확인했다(아니면 redirect 한다).
  // 여기서 다시 읽는 것은 **세션이 아니라 DB 행의 이메일** 때문이다 — 로컬
  // 발급 화면도 같은 이유로 `users` 를 한 번 읽는다.
  const current = await readPageApi<SessionView>("/api/ui/v1/session");

  return (
    <section className={layout.page}>
      <PageHeader title={<>새 인증서</>} />
      <div>
        {/* 서버 액션을 `nodeId` 로 묶어 폼에 넘긴다 — 폼이 보는 시그니처는
            `(prev, formData)` 그대로라 로컬 액션과 구분되지 않는다. */}
        <CertificateForm
          info={
            <>
              이 노드에서 Let&apos;s Encrypt 인증서를 발급받습니다. 검증 방식에
              따라 필요한 준비가 다릅니다.
            </>
          }
          target={{ kind: "node", id, name: auth.node.name }}
          action={{ id: "node.createNodeCertificate", args: [id] }}
          defaultEmail={current?.user.email ?? ""}
          cancelHref={basePath}
        />
      </div>
    </section>
  );
}
