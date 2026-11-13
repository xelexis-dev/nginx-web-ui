import { PageHeader } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { configPreview, type ConfigPreview } from "@/lib/config-preview";
import { can } from "@/lib/permissions";
import {
  ServerDetail,
  type ServerDetailVM,
} from "@/components/reverse-proxy/server-detail";
import {
  storedConfig,
  type ServerSourceRow,
} from "@/components/reverse-proxy/server-vm";
import type { CertificateSourceRow } from "@/components/reverse-proxy/server-form-vm";
import { loadNodePageAll } from "@/lib/node-page";

// 노드의 리버스 프록시 상세. 화면 본문은
// components/reverse-proxy/server-detail.tsx — 로컬 상세와 같은 컴포넌트다.
//
// **엔드포인트를 셋 부른다**(로컬 상세가 DB 를 세 번 읽는 것과 같은 이유):
// 서버 한 행 + upstream 전부(대상 라벨과 참조 블록) + 인증서(TLS 라벨과 블록).
// 인증서는 **목록**으로 받는다 — 어느 인증서가 필요한지는 서버 행을 받아봐야
// 아는데, `loadNodePageAll` 의 경로 집합은 호출 시점에 정해진다. 한 번 더
// 왕복하는 대신 목록에서 골라 쓴다(Task 5 의 수정 화면도 같은 목록이 필요하다).
//
// **라우트 파라미터 이름에 주의.** `[id]` 는 노드, `[sid]` 는 이 화면이 보는
// 서버다 — 둘 다 `[id]` 로 두면 Next 가 라우트를 거부하고, 이름을 헷갈리면
// 조용히 엉뚱한 걸 불러온다(노드 stream·upstream·감사 상세와 같은 관용).
//
// 로컬은 행이 없으면 `notFound()` 지만 여기서는 노드가 404 를 내고
// `NodeErrorCard` 의 `not_found` 카드가 뜬다 — 스펙 §9 가 정한 모양이다.
export const runtime = "nodejs";

/** `GET /api/v1/servers/:id` 의 `server`. 노드는 `nginx_servers` 행을 그대로 준다. */
interface NodeServerDetailResponse {
  preview?: ConfigPreview;
  server: ServerSourceRow & { certificateId: string | null };
}

/** `GET /api/v1/upstreams` 의 응답. `servers` 는 jsonb 라 배열이 아닐 수도 있다. */
interface NodeUpstreamsResponse {
  upstreams: {
    id: string;
    name: string;
    servers: unknown;
    resolveAtRuntime?: boolean;
  }[];
}

/**
 * `GET /api/v1/certificates` 의 응답 — **원시 행이 아니라 허용목록**이다
 * (`api/v1/certificates/view.ts`). `certPem`/`keyPem` 은 일부러 빠져 있어
 * TLS 판정은 `status` 까지만 본다(`ServerDetailCertVM.tlsReady` 문서).
 *
 * 한 항목의 모양은 **`CertificateSourceRow` 하나로 선언한다** — 이 화면과
 * 생성·수정 폼 셋이 같은 응답을 읽는데 각자 적으면 계약이 바뀌는 날 한 곳만
 * 고쳐진다(그래서 그 타입에 이름을 붙였다).
 */
interface NodeCertificatesResponse {
  certificates: CertificateSourceRow[];
}

export default async function NodeServerDetailPage({
  params,
}: {
  params: Promise<{ id: string; sid: string }>;
}) {
  const { id, sid } = await params;
  const basePath = `/nodes/${id}/reverse-proxy`;

  const loaded = await loadNodePageAll<
    "server" | "upstreams" | "certificates",
    ServerDetailVM
  >({
    nodeId: id,
    action: "config:read",
    paths: {
      server: `/api/v1/servers/${encodeURIComponent(sid)}`,
      upstreams: "/api/v1/upstreams",
      certificates: "/api/v1/certificates",
    },
    // **여기가 이 이름들이 값을 갖는 자리다.** 조회 셋 중 무엇이 404 인지 카드가
    // 말하지 않으면, 인증서를 못 읽은 것을 사용자는 「이 서버가 사라졌다」로
    // 읽는다(`NodeErrorCardProps.subject` 문서).
    subjects: {
      server: "그 서버를",
      upstreams: "Upstream 목록을",
      certificates: "인증서 목록을",
    },
    // 「다시 시도」는 이 화면, 「목록으로 돌아가기」는 목록이다. 이 항목이
    // 사라져서 카드가 뜬 것이므로 되돌려 보내면 같은 카드가 다시 뜬다.
    retryHref: `${basePath}/${sid}`,
    listHref: basePath,
    toViewModel: (raw) => {
      const { server, preview } = raw.server as NodeServerDetailResponse;
      const { upstreams } = raw.upstreams as NodeUpstreamsResponse;
      const { certificates } = raw.certificates as NodeCertificatesResponse;

      // 연결된 인증서가 목록에 없으면 `null` — 로컬에서 「연결된 인증서가
      // 삭제됨」으로 그려지는 그 상태다(공유 컴포넌트가 같은 분기를 쓴다).
      const cert = server.certificateId
        ? (certificates.find((c) => c.id === server.certificateId) ?? null)
        : null;

      return {
        id: server.id,
        preview: configPreview(preview),
        serverName: server.serverName,
        listen: server.listen,
        enabled: server.enabled,
        config: storedConfig(server.config),
        upstreamId: server.upstreamId,
        certificateId: server.certificateId,
        upstreams: upstreams.map((u) => ({
          id: u.id,
          name: u.name,
          resolveAtRuntime: u.resolveAtRuntime,
          servers: Array.isArray(u.servers) ? (u.servers as string[]) : [],
        })),
        cert: cert
          ? {
              id: cert.id,
              domains: Array.isArray(cert.domains) ? cert.domains : [],
              staging: cert.staging,
              issued: cert.status === "issued",
              // 계약에 PEM 이 없다 — 왜 이것으로 충분한지는 `tlsReady` 문서.
              tlsReady: cert.status === "issued",
            }
          : null,
        // **노드에서는 항상 false 다.** 판정에 필요한 「집계 제외 도메인」이
        // `/api/v1` 에 없고, 안내가 가리키는 설정 화면은 게이트웨이 자신의
        // 것이라 노드 화면에서 띄우면 엉뚱한 서버를 고치라고 말하게 된다
        // (`ServerDetailVM.selfProxyNotice` 문서).
        selfProxyNotice: false,
      };
    },
  });

  if (!loaded.ok)
    return (
      <section className={layout.page}>
        <PageHeader title="리버스 프록시" />
        {loaded.render}
      </section>
    );

  // 수정·토글·삭제를 보여줄지는 상세를 읽는 데 필요한 `config:read` 와 별개다.
  // `resolveNode` 가 계산한 `effectiveRole`(= min(사용자, 토큰), 스펙 §8)을
  // 그대로 쓴다 — 손으로 재조립하지 않는다(Global Constraints).
  const canWrite = can(loaded.effectiveRole, "config:write");

  return (
    <ServerDetail
      runtimeDns={loaded.runtimeDns}
      data={loaded.data}
      basePath={basePath}
      // 액션은 `nodeId` 로 묶어 넘긴다 — 폼이 보는 시그니처는 `(prev, formData)`
      // 그대로라 공유 컴포넌트는 로컬 액션과 구분하지 못한다.
      write={
        canWrite
          ? {
              editHref: `${basePath}/${sid}/edit`,
              toggleAction: { id: "node.toggleNodeServer", args: [id] },
              deleteAction: { id: "node.deleteNodeServer", args: [id] },
            }
          : null
      }
      configOrigin="remote-node"
    />
  );
}
