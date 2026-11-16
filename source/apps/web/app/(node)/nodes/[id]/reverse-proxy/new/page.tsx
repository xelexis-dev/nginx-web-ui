import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { ServerForm } from "@/components/reverse-proxy/server-form";
import {
  toCertificateOptions,
  type CertificateSourceRow,
} from "@/components/reverse-proxy/server-form-vm";
import { loadNodePageAll } from "@/lib/node-page";

// 노드에 리버스 프록시 서버를 새로 만드는 화면. 폼 본문은
// components/reverse-proxy/server-form.tsx — 로컬 `(app)/reverse-proxy/new` 와
// 같은 컴포넌트다.
//
// **upstream 화면의 `new` 와 달리 `authorizeNodePage` 로는 부족하다.** 이 폼은
// 두 개의 후보 목록(upstream · 인증서)을 채워야 하고, **그 둘은 노드 것이어야
// 한다**: 게이트웨이 자기 목록을 실으면 사용자가 게이트웨이의 upstream 을 골라
// 저장하게 되고, 노드의 `buildServerRow` 가 그 id 를 자기 DB 에서 못 찾아
// 「upstream을 선택하세요」로 거절하거나(locations 모드), 단일 모드면 컬럼에
// 그대로 들어가 **어디도 가리키지 않는 참조**가 된다.
//
// **템플릿 프리필(`?template=`)은 없다.** 템플릿 갤러리(`/reverse-proxy/templates`)
// 가 게이트웨이 로컬 라우트라 노드 목록 화면은 그 버튼을 아예 그리지 않는다
// (`ServerListWrite.templatesHref = null`) — 들어올 링크가 없는 질의 파라미터를
// 읽으면 「어디서도 닿을 수 없는 분기」가 하나 늘 뿐이다.
export const runtime = "nodejs";

/** `GET /api/v1/upstreams` 의 응답. 노드는 `upstreams` 테이블 행을 그대로 준다. */
interface NodeUpstreamsResponse {
  upstreams: { id: string; name: string; resolveAtRuntime?: boolean }[];
}

/**
 * `GET /api/v1/certificates` 의 응답 — 원시 행이 아니라 허용목록이다
 * (`api/v1/certificates/view.ts`). 폼의 TLS Select 가 쓰는 넷이 전부 있다.
 */
interface NodeCertificatesResponse {
  certificates: CertificateSourceRow[];
}

interface NewServerFormData {
  upstreams: { id: string; name: string; resolveAtRuntime?: boolean }[];
  certificates: ReturnType<typeof toCertificateOptions>;
}

export default async function NewNodeServerPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const basePath = `/nodes/${id}/reverse-proxy`;

  const loaded = await loadNodePageAll<
    "upstreams" | "certificates",
    NewServerFormData
  >({
    nodeId: id,
    // 읽기지만 **`config:write` 로 연다**: 쓰기 화면이라 그 등급이 없으면 폼을
    // 띄우는 것 자체가 헛일이다(로컬 `new/page.tsx` 의 `requireRole("config:write")`
    // 에 대응). 노드 쪽 GET 라우트는 `config:read` 만 요구하므로 이 판정은
    // 게이트웨이가 화면을 가리기 위한 것이다(스펙 §8).
    action: "config:write",
    paths: {
      upstreams: "/api/v1/upstreams",
      certificates: "/api/v1/certificates",
    },
    // 카드 하나가 조회 둘을 대표한다 — 이름이 없으면 사용자가 「이 화면이
    // 사라졌다」로 읽는다(`LoadNodePageAllArgs.subjects` 문서).
    subjects: { upstreams: "Upstream 목록을", certificates: "인증서 목록을" },
    retryHref: `${basePath}/new`,
    listHref: basePath,
    // 응답 모양이 어긋나면 여기서 던지고 `loadNodePageAll` 이 `node_error` 카드로
    // 접는다 — 방어 코드를 두지 않는다(Global Constraints).
    toViewModel: (raw) => {
      const { upstreams } = raw.upstreams as NodeUpstreamsResponse;
      const { certificates } = raw.certificates as NodeCertificatesResponse;
      return {
        upstreams: upstreams.map((u) => ({
          id: u.id,
          name: u.name,
          resolveAtRuntime: u.resolveAtRuntime,
        })),
        // **발급 완료만 후보다.** 로컬은 SQL 에서 거르지만 노드는 목록 전체가
        // 오므로 여기서 같은 함수로 거른다(`toCertificateOptions` 문서).
        certificates: toCertificateOptions(certificates),
      };
    },
  });

  if (!loaded.ok)
    return (
      <section className={layout.page}>
        <PageHeader title="새 서버" />
        {loaded.render}
      </section>
    );

  return (
    <section className={layout.page}>
      <PageHeader
        title="새 서버"
        info="새 리버스 프록시 서버를 등록합니다."
      />

      {/* 서버 액션을 `nodeId` 로 묶어 폼에 넘긴다 — 폼이 보는 시그니처는
          `(prev, formData)` 그대로라 로컬 액션과 구분되지 않는다. */}
      <ServerForm
        target={{ kind: "node", id, name: loaded.node.name }}
        nodeName={loaded.node.name}
        legacyNode={/^1\.19\.\d+/.test(loaded.node.lastVersion ?? "")}
        runtimeDns={loaded.runtimeDns}
        action={{ id: "node.createNodeServer", args: [id] }}
        upstreams={loaded.data.upstreams}
        certificates={loaded.data.certificates}
        cancelHref={basePath}
      />
    </section>
  );
}
