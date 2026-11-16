import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { redirect } from "next/navigation";
import {
  CertificateEditForm,
  type CertificateEditFormProps,
} from "@/components/certificates/certificate-edit-form";
import {
  readAlias,
  readCertificateSource,
  readNullableText,
  readText,
  type CertificateChallengeType,
  type CertificateSource,
} from "@/components/certificates/certificate-vm";
import { readPageApi } from "@/lib/api-page";
import type { SessionView } from "@repo/contracts";
import { loadNodePageAll } from "@/lib/node-page";

// 노드의 인증서 설정 수정 화면. 폼 본문은
// components/certificates/certificate-edit-form.tsx — 로컬
// `(app)/certificates/[id]/edit` 와 같은 컴포넌트다.
//
// **엔드포인트는 하나다**(인증서 한 건). 로컬은 저장된 ACME 계정 주소를 읽으려고
// DB 를 한 번 더 보는데, 노드에서는 그 값이 이 응답에 함께 온다(`acmeEmail`).
//
// **Cloudflare 토큰 원문은 게이트웨이로 오지 않는다.** 계약은 `hasDnsToken:
// boolean` 만 내려주고(`api/v1/certificates/view.ts`), 폼은 「값을 안 보내면
// 유지」 표식(`KEEP_DNS_TOKEN`)으로 그 상태를 다룬다. 그래서 이 화면을 여는
// 것만으로 노드의 자격증명이 브라우저나 게이트웨이로 새지 않는다.
//
// ── 연락 이메일은 **노드에 저장된 주소가 우선**이다 ────────────────────────
//
// 로컬 수정 화면과 같은 순서다(`(app)/certificates/[id]/edit/page.tsx`):
// 그 인증서 대상(운영/스테이징)의 저장된 ACME 계정 주소 → 없으면 지금 로그인한
// 사람의 주소. 노드의 그 값은 `certificateView.acmeEmail` 로 온다(1.16.0).
//
// **순서를 뒤집으면 무변경 저장이 무변경이 아니게 된다.** 게이트웨이 사용자의
// 주소로 폼을 채우면, 챌린지 방식만 바꾸고 저장해도 노드의
// `acme_accounts.email` 이 그 사람 주소로 덮인다 — `getOrCreateAccount`
// (`lib/acme-service.ts` 의 「연락 이메일은 계정키와 달리 갱신 대상이다」)가
// 이메일이 다르면 **의도적으로 갱신**하기 때문이다. 그러면 그 노드의 같은
// 대상을 쓰는 **모든 인증서**의 만료·갱신 안내가 그 사람에게 가고, 화면에는
// 아무 표시도 없다.
//
// (계획 4b Task 7 은 처음에 「`getOrCreateAccount` 는 기존 계정의 이메일을
// 건드리지 않으니 무해하다」는 전제로 반대 순서를 썼다. 그 전제는 `7abe9eb`
// 이전의 동작이었다 — 리뷰가 잡았고 계약 한 줄이 더해졌다.)
//
// **라우트 파라미터 이름에 주의.** `[id]` 는 노드, `[cid]` 는 이 화면이 고치는
// 인증서다(상세 화면과 같은 관용).
export const runtime = "nodejs";

/** `GET /api/v1/certificates/:id` 의 `certificate` 중 이 폼이 쓰는 것만. */
interface NodeCertificateResponse {
  certificate: {
    domains: string[];
    /** 별칭을 모르는 구 노드에는 키 자체가 없다 — 그래서 선택. */
    alias?: string | null;
    status: string;
    source: CertificateSource;
    staging: boolean;
    challengeType: CertificateChallengeType;
    dnsProvider: string | null;
    /** 토큰이 **있는지**만 온다 — 원문은 계약에서 빠져 있다. */
    hasDnsToken: boolean;
    /**
     * 노드에 **저장된** ACME 연락 주소(그 대상의 계정이 아직 없으면 `null`).
     * 폼의 기본값이 되는 값이라 위 헤더의 순서대로 여기가 우선이다.
     */
    acmeEmail: string | null;
  };
}

interface EditCertificateFormData {
  /**
   * **`email` 은 여기 없다.** 노드가 준 주소(`acmeEmail`)와 게이트웨이 세션의
   * 주소가 **합쳐져** 폼 기본값이 되는데, 세션은 이 변환 밖에서만 읽을 수 있다.
   * 둘을 합치는 자리는 아래 렌더다.
   */
  defaults: Omit<CertificateEditFormProps["defaults"], "email">;
  /** 노드에 저장된 연락 주소. 폼 기본값의 **1순위**(위 헤더). */
  acmeEmail: string | null;
  isIssued: boolean;
  hasStoredToken: boolean;
  /** `"acme"` 가 아니면 이 화면이 존재하지 않는다 — 아래에서 교체 화면으로 보낸다. */
  source: CertificateSource;
}

export default async function EditNodeCertificatePage({
  params,
}: {
  params: Promise<{ id: string; cid: string }>;
}) {
  const { id, cid } = await params;
  const basePath = `/nodes/${id}/certificates`;

  const loaded = await loadNodePageAll<"certificate", EditCertificateFormData>({
    nodeId: id,
    // 읽기지만 `config:write` 로 연다 — 쓰기 화면이라 그 등급이 없으면 폼을
    // 띄우는 것 자체가 헛일이다(로컬 수정 화면의 `requireRole("config:write")`).
    action: "config:write",
    paths: { certificate: `/api/v1/certificates/${encodeURIComponent(cid)}` },
    subjects: { certificate: "그 인증서를" },
    // 「다시 시도」는 이 수정 화면, 「목록으로 돌아가기」는 목록이다.
    retryHref: `${basePath}/${cid}/edit`,
    listHref: basePath,
    toViewModel: (raw) => {
      const { certificate } = raw.certificate as NodeCertificateResponse;

      return {
        defaults: {
          // **원소 깊이로 읽는다.** 폼이 `defaults.domains.join("\n")` 으로
          // textarea 초기값을 만들고 그 문자열이 그대로 화면에 들어간다 —
          // `join` 은 배열만 보고 원소가 객체여도 `[object Object]` 를 만든다
          // (`certificate-vm.ts` 헤더).
          domains: certificate.domains.map((d, i) =>
            readText(d, `domains[${i}]`),
          ),
          // **읽는다** — 입력의 `defaultValue` 가 되어 그대로 다시 제출된다. 구 노드는
          // 키가 없고 별칭 없는 행은 null 이다 — `readAlias` 가 둘 다 「없음」으로 읽고
          // 폼은 빈 입력란으로 연다.
          alias: readAlias(certificate.alias, "alias") ?? "",
          staging: certificate.staging,
          challengeType:
            certificate.challengeType === "dns-01" ? "dns-01" : "http-01",
          dnsProvider:
            certificate.dnsProvider === "cloudflare" ? "cloudflare" : "",
        },
        // **읽는다** — 이 값은 이메일 입력의 `defaultValue` 가 되어 화면에
        // 나가고 그대로 다시 제출된다. 문자열이 아닌 것이 오면 폼에
        // 「[object Object]」가 들어앉고 저장하는 순간 노드의 연락처가 그걸로
        // 바뀐다.
        acmeEmail: readNullableText(certificate.acmeEmail, "acmeEmail"),
        isIssued: certificate.status === "issued",
        hasStoredToken: certificate.hasDnsToken,
        // **읽는다** — 화면에 나가지는 않지만 아래 `:!== "acme"` 가 **어느 화면을
        // 보여줄지**를 이 값으로 정한다. 그냥 통과시키면 노드가 이상한 값을 줬을 때
        // 「acme 가 아니다」가 참이 되어 **말없이 교체 화면으로 보내진다** — §9 카드가
        // 떠야 할 자리에서 멀쩡해 보이는 다른 화면이 뜬다. 렌더되지 않아도 **분기를
        // 고르는 값**이면 관문에서 읽어야 한다는 규칙이 여기에도 걸린다.
        source: readCertificateSource(certificate.source, "source"),
      };
    },
  });

  if (!loaded.ok) return (
    <section className={layout.page}>
      <PageHeader title="인증서 수정" />
      {loaded.render}
    </section>
  );

  // **수동 등록 인증서에는 발급 설정이라는 게 없다.** 그 대신 있는 것이 교체
  // 화면이고, 로컬 수정 화면과 **같이** 그리로 보낸다
  // (`(app)/certificates/[id]/edit/page.tsx` 의 `redirect(.../replace)`).
  // 상세의 액션 행도 수동 인증서에는 「설정 수정」을 그리지 않으므로 여기까지
  // 오는 길은 주소를 직접 친 경우뿐인데, 그때 404 로 막는 것보다 원하는 화면으로
  // 보내는 것이 맞다. 노드가 PATCH 를 409 로 거절하기도 하지만
  // (`updateCertificateRow`), 그건 저장을 눌러야 보인다.
  if (loaded.data.source !== "acme") redirect(`${basePath}/${cid}/replace`);

  // 폼 기본값의 **2순위**만 여기서 온다 — 노드에 그 대상의 ACME 계정이 아직
  // 없을 때(`acmeEmail === null`)뿐이다. 로컬 수정 화면과 같은 순서다.
  const current = await readPageApi<SessionView>("/api/ui/v1/session");

  return (
    <section className={layout.page}>
      <PageHeader
        title={<>인증서 수정</>}
        info="발급 설정을 바꿉니다."
      />
      <CertificateEditForm
        info={
          <>
            인증서 ID는 그대로라 <strong>연결된 서버는 그대로 유지</strong>
            됩니다. 저장은 설정만 바꾸며, 실제 재발급은 상세 화면에서
            실행합니다.
          </>
        }
        target={{ kind: "node", id, name: loaded.node.name }}
        action={{ id: "node.updateNodeCertificate", args: [id] }}
        id={cid}
        cancelHref={`${basePath}/${cid}`}
        isIssued={loaded.data.isIssued}
        hasStoredToken={loaded.data.hasStoredToken}
        // **저장된 주소가 먼저다.** 뒤집으면 무변경 저장이 노드의 연락처를
        // 조용히 덮는다(위 헤더) — 로컬과 같은 순서로 둔다.
        defaults={{
          ...loaded.data.defaults,
          email: loaded.data.acmeEmail ?? current?.user.email ?? "",
        }}
      />
    </section>
  );
}
