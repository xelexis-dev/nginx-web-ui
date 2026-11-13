import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { ImportForm } from "@/components/certificates/import-form";
import { readPageView } from "@/lib/api-page";

// 로컬 인증서 가져오기(수동 PEM 등록). 폼 본문은
// components/certificates/import-form.tsx — 노드 화면
// (`(node)/nodes/[id]/certificates/import`)과 같은 컴포넌트를 쓴다.
//
// **액션과 취소 경로를 명시한다.** 추출 전에는 폼이 `importCertificate` 를
// 기본값으로 들고 있었지만, 공유 컴포넌트는 라우트 그룹의 TS 를 가져오지 않는다
// (`certificate-list.tsx` 헤더의 불변식) — 화면이 넘긴다. 값은 추출 전과 같다.
export const runtime = "nodejs";

export default async function ImportCertificatePage() {
  await readPageView("/certificates/import");
  return (
    <section className={layout.page}>
      <PageHeader title={<>인증서 가져오기</>} />
      <ImportForm
        info={
          <>
            보유한 PEM 인증서와 개인키를 붙여넣거나 파일로 업로드해 등록합니다.
            도메인·만료일은 인증서에서 자동 추출됩니다.
          </>
        }
        action={{ id: "certificates.importCertificate", args: [] }}
        cancelHref="/certificates"
      />
    </section>
  );
}
