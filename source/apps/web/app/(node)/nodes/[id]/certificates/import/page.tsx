import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { ImportForm } from "@/components/certificates/import-form";
import { authorizeNodePage } from "@/lib/node-page";

// 노드에 수동 PEM 인증서를 등록하는 화면. 폼 본문은
// components/certificates/import-form.tsx — 로컬 `(app)/certificates/import`
// 와 같은 컴포넌트다.
//
// **노드에서 읽어오는 것이 없다** — 이 폼은 후보 목록을 채우지 않는다(발급
// 화면과 같다). 그래서 `loadNodePageAll` 이 아니라 `authorizeNodePage` 로 접근만
// 연다: 부를 경로가 없는데 `loadNodePage*` 를 쓰면 실패 카드를 위해 아무 종점이나
// 골라 부르게 된다.
//
// ── ⚠ 이 화면은 **개인키를 노드로 보낸다** ──────────────────────────────────
//
// 노드 설계의 방향은 「노드의 비밀값은 노드에 남는다」인데(Cloudflare 토큰은
// `hasDnsToken: boolean` 으로만 올라오고, 발급된 인증서의 `certPem`/`keyPem` 은
// 계약에서 빠져 있다) **이 화면만 반대 방향**이다: 운영자가 붙여넣은 키가
// 게이트웨이를 지나 노드로 간다.
//
// 그 키가 게이트웨이에 **머무는 자리가 없다**는 것이 이 화면의 불변식이고,
// 근거는 두 곳에 적혀 있다 — 폼 쪽은 `ImportForm` 헤더, 서버 쪽은
// `actions.ts` 의 「수동 PEM」 절(감사·탐침·로그·반환 상태 넷 다).
//
// **CSS 는 라우트 그룹 시트를 가져다 쓴다** — 이유는
// `components/certificates/certificate-list.tsx` 헤더(요약: `composes` 가 걸려
// 있고 로컬에 남는 형제 화면들이 같은 클래스를 쓴다 — 옮길 수 없다). 노드
// 수정 화면(`[cid]/edit/page.tsx`)이 같은 판정을 이미 했다.
export const runtime = "nodejs";

export default async function ImportNodeCertificatePage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const basePath = `/nodes/${id}/certificates`;

  const auth = await authorizeNodePage({
    nodeId: id,
    // 읽기가 아니라 쓰기 화면이다 — 그 등급이 없으면 폼을 띄우는 것 자체가
    // 헛일이다(로컬 `import/page.tsx` 의 `requireRole("config:write")` 와
    // 노드 라우트의 `requireNodeAction(caller, "config:write")` 둘 다 이 등급).
    action: "config:write",
    retryHref: `${basePath}/import`,
    listHref: basePath,
  });
  if (!auth.ok) return (
    <section className={layout.page}>
      <PageHeader title="인증서 가져오기" />
      {auth.render}
    </section>
  );

  return (
    <section className={layout.page}>
      <PageHeader title={<>인증서 가져오기</>} />
      {/* 서버 액션을 `nodeId` 로 묶어 폼에 넘긴다 — 폼이 보는 시그니처는
          `(prev, formData)` 그대로라 로컬 액션과 구분되지 않는다. */}
      <ImportForm
        info={
          <>
            보유한 PEM 인증서와 개인키를 붙여넣거나 파일로 업로드해 이 노드에
            등록합니다. 도메인·만료일은 인증서에서 자동 추출됩니다.
          </>
        }
        target={{ kind: "node", id, name: auth.node.name }}
        action={{ id: "node.importNodeCertificate", args: [id] }}
        cancelHref={basePath}
      />
    </section>
  );
}
