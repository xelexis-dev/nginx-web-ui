import Link from "next/link";
import { PROXY_TABS } from "@/app/(app)/nav-links";
import { PageHeader, SectionNav } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { can } from "@/lib/permissions";
import { readConfigDiff, type ConfigDiff } from "@/lib/config-diff";
import {
  ServerList,
  type ServerListVM,
} from "@/components/reverse-proxy/server-list";
import type { ServerSourceRow } from "@/components/reverse-proxy/server-vm";
import { readNodeCertificateSummaries } from "@/components/reverse-proxy/certificate-read";
import {
  certificateIndex,
  pendingIndex,
  toOpsRow,
} from "@/components/reverse-proxy/server-status";
import { loadNodePageAll } from "@/lib/node-page";

// 노드의 리버스 프록시 목록 + 「설정 적용」. 화면 본문은
// components/reverse-proxy/server-list.tsx — 로컬 (app)/reverse-proxy/page.tsx 와
// 같은 컴포넌트를 쓴다.
//
// **인증서는 따로 읽는다**(TLS 칸) — 노드 인증서 화면의 뷰를 보조 읽기로 한 번 더 부른다
// (`components/reverse-proxy/certificate-read.ts`). 못 읽으면 TLS 칸만 「확인 불가」다.
//
// **엔드포인트를 셋 부른다.** upstream 목록(대상 열의 이름) + 서버 목록 +
// **미적용 변경**(`/api/v1/pending`). 마지막을 빼고 게이트웨이 자신의
// `getPendingStatus()` 를 쓰면 「적용할 게 없는데 배지가 떠 있다」(반대도) 가
// 된다 — 그 배지가 말하는 것은 **노드**의 미적용 변경이다. upstream 도 노드
// 것이어야 한다: 게이트웨이 자기 목록으로 이름을 붙이면 대상 열에 남의 upstream
// 이름이 뜨거나 id 가 그대로 노출된다.
//
// **경로가 `/nodes/<id>/reverse-proxy` 인 것이 요건이다** — 로컬 `/reverse-proxy`
// 와 꼬리가 같아야 공유 컴포넌트의 `basePath` 치환이 한 번으로 끝나고(행 링크·
// Upstreams·「+ 새 서버」가 전부 거기서 갈린다), 사이드바 강조도 최장 접두사
// (`(app)/nav-links.ts` 의 `activeHref`)로 로컬과 같게 동작한다.
export const runtime = "nodejs";

/** `GET /api/v1/upstreams` 의 한 행. 노드는 `upstreams` 테이블 행을 그대로 준다. */
interface NodeUpstreamsResponse {
  upstreams: { id: string; name: string; resolveAtRuntime?: boolean }[];
}

/** `GET /api/v1/servers` 의 응답. 노드는 `nginx_servers` 행을 그대로 준다(`api/v1/servers/route.ts`). */
interface NodeServersResponse {
  servers: ServerSourceRow[];
}

/** 노드 목록 읽기의 결과 — 행은 인증서 보조 읽기가 끝난 뒤에 만든다. */
interface NodeListRaw {
  servers: ServerSourceRow[];
  upstreamNameById: Map<string, string>;
  pendingCount: number;
  pendingHasBaseline: boolean;
  pendingDiff: ConfigDiff;
}

/** `GET /api/v1/pending` 의 응답(`lib/pending.ts` 의 `PendingStatus` 그대로). */
interface NodePendingResponse {
  count: number;
  hasBaseline?: boolean;
  diff: ConfigDiff;
}

export default async function NodeReverseProxyPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const basePath = `/nodes/${id}/reverse-proxy`;

  const [loaded, certificates] = await Promise.all([
    loadNodePageAll<"upstreams" | "servers" | "pending", NodeListRaw>({
      nodeId: id,
      action: "config:read",
      paths: {
        upstreams: "/api/v1/upstreams",
        servers: "/api/v1/servers",
        pending: "/api/v1/pending",
      },
      // 카드 하나가 조회 셋을 대표한다 — 이름이 없으면 사용자가 「이 화면이
      // 사라졌다」로 읽는다(`LoadNodePageAllArgs.subjects` 문서).
      subjects: {
        upstreams: "Upstream 목록을",
        servers: "서버 목록을",
        pending: "미적용 변경을",
      },
      retryHref: basePath,
      // 목록 화면이라 둘이 같은 곳이다 — 같은 값을 그대로 넘긴다(우연이 아니라
      // 단언, `NodeErrorCardProps` 문서).
      listHref: basePath,
      // 응답 모양이 어긋나면 여기서 던지고 `loadNodePageAll` 이 `node_error` 카드로
      // 접는다 — 방어 코드를 두지 않는다(Global Constraints).
      toViewModel: (raw) => {
        const { upstreams } = raw.upstreams as NodeUpstreamsResponse;
        const { servers } = raw.servers as NodeServersResponse;
        const pending = raw.pending as NodePendingResponse;
        // **노드의 upstream 이름으로 대상 라벨을 만든다** — 로컬 목록과 같은 함수다.
        const upstreamNameById = new Map(upstreams.map((u) => [u.id, u.name]));
        return {
          servers,
          upstreamNameById,
          // **노드가 준 값이다.** 게이트웨이의 `getPendingStatus()` 를 부르지 마라.
          pendingCount: pending.count,
          pendingHasBaseline: pending.hasBaseline === true,
          // **`readConfigDiff` 가 화면과 같은 깊이로 읽는다.** 예전에는
          // `.length` 셋을 더하는 것으로 「읽었다」고 봤는데, **문자열에도
          // `.length` 가 있어** 통과했고 실패는 모달이 `.map` 하는 렌더 도중에
          // 터졌다 — §9 카드가 아니라 일반 500 화면이다. 배지 뒤에 접혀 있어도
          // 소용없다(`Modal` 은 닫혀 있어도 children 을 렌더한다).
          pendingDiff: readConfigDiff(pending.diff),
        };
      },
    }),
    readNodeCertificateSummaries(id),
  ]);

  if (!loaded.ok)
    return (
      <section className={layout.page}>
        <PageHeader title="리버스 프록시" />
        {loaded.render}
      </section>
    );

  // 행은 두 읽기가 다 끝난 뒤에 만든다 — TLS 칸은 인증서, 미적용 표시는 노드의 diff 에서 온다.
  const {
    servers,
    upstreamNameById,
    pendingCount,
    pendingHasBaseline,
    pendingDiff,
  } = loaded.data;
  const certs = certificateIndex(certificates);
  const pending = pendingIndex(pendingDiff);
  const list: Omit<ServerListVM, "canApply"> = {
    items: servers.map((s) => toOpsRow(s, upstreamNameById, certs, pending)),
    certificatesUnknown: certs === null,
    pendingCount,
    pendingHasBaseline,
    pendingDiff,
  };

  // 적용 바와 쓰기 UI 는 목록을 읽는 데 필요한 `config:read` 와 별개로, 그리고
  // 서로도 별개로 판정한다(editor 는 둘 다, viewer 는 둘 다 없다 — 다만 등급
  // 매트릭스가 바뀌면 갈릴 수 있는 자리라 조건을 하나로 합치지 않는다).
  // **`effectiveRole` 을 손으로 다시 만들지 않는다** — `resolveNode` 가 이미
  // min(사용자, 토큰)을 계산해 실어 준다(`NodePageData` 문서).
  const canApply = can(loaded.effectiveRole, "nginx:reload");
  const canWrite = can(loaded.effectiveRole, "config:write");

  return (
    <ServerList
      data={{ ...list, canApply }}
      basePath={basePath}
      navigation={<SectionNav items={PROXY_TABS.filter(tab => tab.href !== "/reverse-proxy/templates").map(tab => ({ ...tab, href: `/nodes/${id}${tab.href}` }))} currentPath={basePath} label="프록시 구성" renderLink={link => <Link {...link} />} />}
      // **템플릿 화면은 노드에 없다.** `(app)/reverse-proxy/templates` 는
      // 게이트웨이 로컬 라우트이고(그 화면이 자기 DB 도 안 본다는 뜻이 아니라
      // 노드 경로 아래에 같은 라우트가 없다는 뜻이다), 노드 경로로 링크를 만들면
      // 404 다. `templatesHref: null` 이면 그 버튼만 사라진다.
      // 행 스위치는 상세 화면의 토글과 같은 명령을 **이 노드**로 묶어 보낸다.
      write={
        canWrite
          ? {
              newHref: `${basePath}/new`,
              templatesHref: null,
              toggleAction: { id: "node.toggleNodeServer", args: [id] },
            }
          : null
      }
    />
  );
}
