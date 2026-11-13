import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { readPageView } from "@/lib/api-page";
import { ImportForm } from "@/components/certificates/import-form";

// 로컬 인증서 교체(수동 PEM 덮어쓰기). 폼 본문은
// components/certificates/import-form.tsx — 노드 화면
// (`(node)/nodes/[id]/certificates/[cid]/replace`)과 같은 컴포넌트를 쓴다.
//
// **이 화면은 현재 PEM 을 보여주지 않는다** — 추출 전에도 그랬다. 폼의 두
// textarea 는 언제나 빈 값으로 시작한다(`ImportForm` 의 `useState("")`), 그리고
// 이 페이지가 읽는 DB 행은 `source` 확인에만 쓰인다. 그래서 노드 교체 화면과
// **보이는 것이 정확히 같다** — 계약(`certificateView`)이 `certPem`/`keyPem` 을
// 빼는 것이 이 화면에서는 아무것도 가리지 않는다.
export const runtime = "nodejs";

export default async function ReplaceCertificatePage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  await readPageView(`/certificates/${id}/replace`);
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>인증서 교체</>}
        info="새 PEM으로 이 인증서를 교체합니다."
      />
      <ImportForm
        info={<>연결된 서버가 있으면 즉시 재적용됩니다.</>}
        action={{ id: "certificates.replaceCertificate", args: [] }}
        certId={id}
        cancelHref="/certificates"
      />
    </section>
  );
}
