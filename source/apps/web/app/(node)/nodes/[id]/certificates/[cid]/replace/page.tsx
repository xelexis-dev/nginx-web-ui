import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { notFound } from "next/navigation";
import { ImportForm } from "@/components/certificates/import-form";
import {
  readText,
  type CertificateSource,
} from "@/components/certificates/certificate-vm";
import { loadNodePageAll } from "@/lib/node-page";

// 노드의 수동 인증서를 새 PEM 으로 교체하는 화면. 폼 본문은
// components/certificates/import-form.tsx — 로컬
// `(app)/certificates/[id]/replace` 와 같은 컴포넌트다.
//
// **엔드포인트는 하나다**(인증서 한 건). 읽는 이유도 로컬과 같다: **수동 인증서인지
// 확인하는 것**뿐이다.
//
// ── 현재 PEM 은 보여주지 않는다 — **로컬도 그렇다** ─────────────────────────
//
// 계약(`api/v1/certificates/view.ts`)이 `certPem`/`keyPem` 을 허용목록에서 빼므로
// 이 화면은 지금 깔려 있는 인증서 본문을 받아올 수 없다. **그런데 그것이 이
// 화면에서 가리는 것은 아무것도 없다** — 로컬 교체 화면도 현재 PEM 을 그리지
// 않고 두 textarea 를 **빈 값으로** 시작한다(`ImportForm` 의 `useState("")`).
// 로컬이 DB 행을 읽는 것도 `source` 확인 한 가지다. 그래서 두 화면은 **보이는
// 것이 정확히 같다** — 노드 쪽에만 비어 있는 자리가 없다.
//
// (도메인·만료일 같은 현재 값을 여기에 덧붙일 수는 있지만, 그러면 로컬에 없는
// 것이 노드에만 생겨 두 화면이 갈라진다. 필요해지면 **양쪽에** 더할 일이다.)
//
// ── ⚠ 이 화면은 **개인키를 노드로 보낸다** ──────────────────────────────────
//
// 가져오기 화면과 같은 방향이고 같은 불변식이다 — 근거는 `ImportForm` 헤더와
// `actions.ts` 의 「수동 PEM」 절.
//
// **라우트 파라미터 이름에 주의.** `[id]` 는 노드, `[cid]` 는 이 화면이 교체하는
// 인증서다(상세·수정 화면과 같은 관용).
export const runtime = "nodejs";

/** `GET /api/v1/certificates/:id` 의 `certificate` 중 이 화면이 쓰는 것만. */
interface NodeCertificateResponse {
  certificate: {
    source: CertificateSource;
  };
}

interface ReplaceCertificateData {
  /** `"manual"` 이 아니면 이 화면이 존재하지 않는다 — 아래 `notFound()`. */
  source: CertificateSource;
}

export default async function ReplaceNodeCertificatePage({
  params,
}: {
  params: Promise<{ id: string; cid: string }>;
}) {
  const { id, cid } = await params;
  const basePath = `/nodes/${id}/certificates`;

  const loaded = await loadNodePageAll<"certificate", ReplaceCertificateData>({
    nodeId: id,
    // 읽기지만 `nginx:reload` 로 연다 — 교체는 연결된 서버가 있으면 곧바로
    // 재적용되므로 노드 라우트도 그 등급을 요구하고(`[id]/replace/route.ts`),
    // 로컬 교체 화면도 `requireRole("nginx:reload")` 다. 더 낮은 등급으로 열면
    // 폼을 다 채우고 제출한 뒤에야 거부당한다.
    action: "nginx:reload",
    paths: { certificate: `/api/v1/certificates/${encodeURIComponent(cid)}` },
    subjects: { certificate: "그 인증서를" },
    // 「다시 시도」는 이 교체 화면, 「목록으로 돌아가기」는 목록이다.
    retryHref: `${basePath}/${cid}/replace`,
    listHref: basePath,
    toViewModel: (raw) => {
      const { certificate } = raw.certificate as NodeCertificateResponse;

      return {
        // **그려지지 않는데도 관문에서 읽는다.** 이 값은 화면에 안 나가지만
        // **이 화면을 그릴지 404 를 낼지**를 고른다 — 문자열이 아닌 무엇(예:
        // `{}`)이 오면 `!== "manual"` 이 참이라 조용히 「그런 화면이 없다」로
        // 접히고, 사용자는 자기 인증서가 사라졌다고 읽는다. 여기서 읽으면
        // 그 경우가 §9 카드(「노드에서 오류가 발생했습니다」)가 된다
        // (`dnsSubmittedAt` 을 상세에서 읽는 것과 같은 갈래).
        source: readText(certificate.source, "source") as CertificateSource,
      };
    },
  });

  if (!loaded.ok) return loaded.render;

  // **ACME 인증서에는 교체라는 게 없다** — 갱신·재발급이 그 자리다. 로컬 교체
  // 화면의 `cert.source !== "manual"` → `notFound()` 와 같은 판정이다. 노드가
  // POST 를 409(`conflict`)로 거절하기도 하지만, 그건 PEM 을 다 붙여넣고 제출한
  // 뒤에야 보인다.
  if (loaded.data.source !== "manual") notFound();

  return (
    <section className={layout.page}>
      {/* 로컬 교체 화면과 **같은 머리다** — 「← 상세로」 링크가 없는 것도
              같다(수정 화면에는 있다). 나갈 길은 폼의 「취소」이고 그것도 로컬과
              같은 곳(인증서 목록)을 가리킨다. */}
      <PageHeader
        title={<>인증서 교체</>}
        info="새 PEM으로 이 인증서를 교체합니다."
      />
      {/* 서버 액션을 `nodeId` 로 묶어 폼에 넘긴다 — 폼이 보는 시그니처는
          `(prev, formData)` 그대로라 로컬 액션과 구분되지 않는다. */}
      <ImportForm
        info={<>연결된 서버가 있으면 이 노드에서 즉시 재적용됩니다.</>}
        target={{ kind: "node", id, name: loaded.node.name }}
        action={{ id: "node.replaceNodeCertificate", args: [id] }}
        certId={cid}
        cancelHref={basePath}
      />
    </section>
  );
}
