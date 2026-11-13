import layout from "@/components/page-layout.module.css";
import Link from "next/link";
import { Banner, PageHeader, linkClassName } from "@repo/ui";
import { ServerForm } from "@/components/reverse-proxy/server-form";
import {
  toCertificateOptions,
  toServerFormValues,
  type CertificateSourceRow,
  type ServerFormSourceRow,
  type ServerFormValues,
} from "@/components/reverse-proxy/server-form-vm";
import { loadNodePageAll } from "@/lib/node-page";

// 노드의 리버스 프록시 서버 수정 화면. 폼 본문과 **저장된 행 → 폼 초기값 변환**
// 둘 다 로컬 수정 화면(`(app)/reverse-proxy/[id]/edit`)과 공유한다.
//
// **엔드포인트를 셋 부른다** — 이 계획에서 제일 많다. 서버 한 행(초기값) +
// upstream 전부(대상 Select 후보 **그리고** 저장된 `http://<name>` 을 다시
// upstream 모드로 되돌리는 데) + 인증서 전부(TLS Select 후보). 로컬 수정
// 화면이 DB 를 세 번 읽는 것(`[id]/edit/page.tsx`)과 같은 셋이다.
//
// **후보 목록 둘 다 노드 것이어야 한다.** 게이트웨이 자기 목록을 섞으면 저장은
// 되는데 노드에서 참조가 깨진다 — `new/page.tsx` 헤더에 자세히 적었다. 수정
// 화면에는 그 위에 하나가 더 있다: `toServerFormValues` 가 저장된 location 의
// `http://<name>` 을 upstream id 로 되돌릴 때 이 목록을 쓰므로, 남의 목록을
// 주면 **upstream 으로 저장된 location 이 직접 proxy_pass 로 열리고** 사용자가
// 저장하는 순간 그 값이 굳는다.
//
// **라우트 파라미터 이름에 주의.** `[id]` 는 노드, `[sid]` 는 이 화면이 고치는
// 서버다(상세 화면과 같은 관용).
export const runtime = "nodejs";

/** `GET /api/v1/servers/:id` 의 `server`. 노드는 `nginx_servers` 행을 그대로 준다. */
interface NodeServerDetailResponse {
  server: ServerFormSourceRow;
}

/** `GET /api/v1/upstreams` 의 응답. */
interface NodeUpstreamsResponse {
  upstreams: { id: string; name: string; resolveAtRuntime?: boolean }[];
}

/** `GET /api/v1/certificates` 의 응답(허용목록 — `api/v1/certificates/view.ts`). */
interface NodeCertificatesResponse {
  certificates: CertificateSourceRow[];
}

interface EditServerFormData {
  values: ServerFormValues;
  upstreams: { id: string; name: string; resolveAtRuntime?: boolean }[];
  certificates: ReturnType<typeof toCertificateOptions>;
}

export default async function EditNodeServerPage({
  params,
}: {
  params: Promise<{ id: string; sid: string }>;
}) {
  const { id, sid } = await params;
  const basePath = `/nodes/${id}/reverse-proxy`;

  const loaded = await loadNodePageAll<
    "server" | "upstreams" | "certificates",
    EditServerFormData
  >({
    nodeId: id,
    // 읽기지만 `config:write` 로 연다 — 이유는 `new/page.tsx` 와 같다(쓰기
    // 화면이라 그 등급이 없으면 폼을 띄우는 것 자체가 헛일이다).
    action: "config:write",
    paths: {
      server: `/api/v1/servers/${encodeURIComponent(sid)}`,
      upstreams: "/api/v1/upstreams",
      certificates: "/api/v1/certificates",
    },
    // 카드 하나가 조회 셋을 대표한다 — 인증서를 못 읽은 것을 사용자가 「이
    // 서버가 사라졌다」로 읽지 않게 이름을 준다(`subject` 문서).
    subjects: {
      server: "그 서버를",
      upstreams: "Upstream 목록을",
      certificates: "인증서 목록을",
    },
    // 「다시 시도」는 이 수정 화면, 「목록으로 돌아가기」는 목록이다.
    retryHref: `${basePath}/${sid}/edit`,
    listHref: basePath,
    toViewModel: (raw) => {
      const { server } = raw.server as NodeServerDetailResponse;
      const { upstreams } = raw.upstreams as NodeUpstreamsResponse;
      const { certificates } = raw.certificates as NodeCertificatesResponse;

      const options = upstreams.map((u) => ({
        id: u.id,
        name: u.name,
        resolveAtRuntime: u.resolveAtRuntime,
      }));
      return {
        // 로컬 수정 화면과 **같은 함수**다 — 여기서 갈리면 같은 서버가
        // 게이트웨이와 노드에서 다르게 열린다(`toServerFormValues` 문서).
        values: toServerFormValues(server, options),
        upstreams: options,
        certificates: toCertificateOptions(certificates),
      };
    },
  });

  if (!loaded.ok)
    return (
      <section className={layout.page}>
        <PageHeader title="서버 수정" />
        {loaded.render}
      </section>
    );
  const legacyNode = /^1\.19\.\d+/.test(loaded.node.lastVersion ?? "");
  const unsupported =
    legacyNode &&
    loaded.data.values.locations?.some(
      (route) =>
        route.resolveAtRuntime ||
        route.retryOtherInstances ||
        (!route.upstreamId && !route.proxyPass),
    );

  return (
    <section className={layout.page}>
      <PageHeader
        eyebrow={
          <Link
            href={`${basePath}/${sid}`}
            className={linkClassName({ variant: "back" })}
          >
            {loaded.data.values.serverName.split(/\s+/)[0] || "서버"}
          </Link>
        }
        title="서버 수정"
      />

      {unsupported ? (
        <Banner
          tone="warning"
          title="이 노드는 1.19라 이 구성을 편집할 수 없습니다 — 노드를 2.0으로 올리세요"
        />
      ) : (
        <ServerForm
          target={{ kind: "node", id, name: loaded.node.name }}
          nodeName={loaded.node.name}
          legacyNode={legacyNode}
          nodeVersionFamily={legacyNode ? "1.19" : "2.0"}
          runtimeDns={loaded.runtimeDns}
          action={{ id: "node.updateNodeServer", args: [id] }}
          upstreams={loaded.data.upstreams}
          certificates={loaded.data.certificates}
          defaultValues={loaded.data.values}
          cancelHref={`${basePath}/${sid}`}
        />
      )}
    </section>
  );
}
