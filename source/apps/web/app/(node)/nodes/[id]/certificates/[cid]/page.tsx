import { PageHeader } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { can } from "@/lib/permissions";
import { CertificateDetail } from "@/components/certificates/certificate-detail";
import { CertificateDetailActions } from "@/components/certificates/certificate-detail-actions";
import { DnsChallengePanel } from "@/components/certificates/dns-challenge-panel";
import {
  readAlias,
  readInstant,
  readNullableText,
  readNumber,
  readText,
  toDomains,
  toIssueLogLines,
  toLinkedServer,
  toNodeDnsOrder,
  type CertificateChallengeType,
  type CertificateDetailVM,
  type CertificateSource,
  type CertificateStatus,
  type DnsChallengeRecordView,
} from "@/components/certificates/certificate-vm";
import { loadNodePageAll } from "@/lib/node-page";

// 노드의 인증서 상세. 화면 본문은
// components/certificates/certificate-detail.tsx — 로컬 상세와 같은 컴포넌트다.
//
// **엔드포인트를 둘 부른다**: 인증서 한 건 + **서버 목록**(「연결된 서버」 표).
// 노드에는 「이 인증서를 쓰는 서버」 종점이 따로 없으므로 목록을 받아 여기서
// 거른다 — 종점을 더하는 것은 계약 변경이다(Global Constraints).
//
// **라우트 파라미터 이름에 주의.** `[id]` 는 노드, `[cid]` 는 이 화면이 보는
// 인증서다 — 둘 다 `[id]` 로 두면 Next 가 라우트를 거부하고, 이름을 헷갈리면
// 조용히 엉뚱한 걸 불러온다(노드 서버·stream·감사 상세와 같은 관용).
//
// 로컬은 행이 없으면 `notFound()` 지만 여기서는 노드가 404 를 내고
// `NodeErrorCard` 의 `not_found` 카드가 뜬다 — 스펙 §9 가 정한 모양이다.
//
// **중단만 서버 액션이 아니다.** Next 는 서버 액션을 클라이언트당 직렬로
// dispatch 하므로 중단을 액션으로 만들면 끊으려는 발급 뒤에 줄을 선다 — 노드에서는
// 갱신 라우트가 최대 300초를 붙들어 더 강하게 성립한다. 그래서 중단만 중계
// 라우트(`app/api/nodes/[id]/certificates/[cid]/cancel`)이고, 그 경로를 아래에서
// **props 로** 넘긴다(`CancelIssuanceButton` 문서).
export const runtime = "nodejs";

/** `GET /api/v1/certificates/:id` 의 `certificate` — `certificateView` 의 허용목록. */
interface NodeCertificateResponse {
  certificate: {
    id: string;
    domains: string[];
    /** 별칭을 모르는 구 노드에는 키 자체가 없다 — 그래서 선택. */
    alias?: string | null;
    status: CertificateStatus;
    source: CertificateSource;
    staging: boolean;
    challengeType: CertificateChallengeType;
    dnsProvider: string | null;
    /** 진행 중 DNS-01 주문의 TXT 안내. 주문이 없으면 `null`. */
    dnsRecords: DnsChallengeRecordView[] | null;
    /**
     * 사용자가 「전파 확인 후 계속」을 눌러 CA 검증이 시작된 시각(1.16.0 부터).
     * **이 화면이 TXT 표를 그릴지 「검증 중단」을 그릴지가 여기서 갈린다** —
     * 갱신 주문은 두 단계 모두 `status: "issued"` 라 `status` 로는 못 가른다.
     */
    dnsSubmittedAt: string | null;
    issuedAt: string | null;
    expiresAt: string | null;
    lastError: string | null;
    lastAttemptAt: string | null;
    failureCount: number;
    issueLog: { at: string; level: "info" | "error"; msg: string }[];
  };
}

/**
 * `GET /api/v1/servers` 의 응답. 노드는 `nginx_servers` 행을 그대로 주고
 * **`serverName` 오름차순**이다(`api/v1/servers/route.ts:15`) — 로컬 상세가
 * `orderBy asc(serverName)` 로 읽는 것과 같은 순서라 여기서 다시 정렬하지 않는다.
 */
interface NodeServersResponse {
  servers: { id: string; serverName: string; listen: number; certificateId: string | null }[];
}

export default async function NodeCertificateDetailPage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string; cid: string }>;
  /**
   * 교체 성공 뒤 `replaceNodeCertificate` 가 붙여 보내는 커버리지 경고 —
   * 로컬 상세와 **같은 쿼리·같은 모양**이다(`(app)/certificates/[id]/page.tsx`).
   */
  searchParams: Promise<{ uncovered?: string }>;
}) {
  const { id, cid } = await params;
  const basePath = `/nodes/${id}/certificates`;
  const uncovered = (await searchParams).uncovered?.split(",").filter(Boolean) ?? [];

  const loaded = await loadNodePageAll<"certificate" | "servers", CertificateDetailVM>({
    nodeId: id,
    action: "config:read",
    paths: {
      certificate: `/api/v1/certificates/${encodeURIComponent(cid)}`,
      servers: "/api/v1/servers",
    },
    // 카드 하나가 조회 둘을 대표한다 — 서버 목록을 못 읽은 것을 사용자가
    // 「이 인증서가 사라졌다」로 읽지 않게 한다(`subjects` 문서).
    subjects: { certificate: "그 인증서를", servers: "서버 목록을" },
    // 「다시 시도」는 이 화면, 「목록으로 돌아가기」는 목록이다. 이 항목이
    // 사라져서 카드가 뜬 것이므로 되돌려 보내면 같은 카드가 다시 뜬다.
    retryHref: `${basePath}/${cid}`,
    listHref: basePath,
    toViewModel: (raw) => {
      const { certificate } = raw.certificate as NodeCertificateResponse;
      const { servers } = raw.servers as NodeServersResponse;

      return {
        id: certificate.id,
        // 아래 넷(`toDomains`·`toIssueLogLines`·`toNodeDnsOrder`·`readInstant`)은
        // **옮겨 담기가 아니라 읽기**다 — 화면이 `join`/`replace`/`toISOString`
        // 을 부르는 깊이 그대로 관문 안에서 한 번 부른다. 통과만 시키면 그
        // 실패가 렌더 도중에 터져 §9 카드가 아니라 일반 500 화면이 된다
        // (`certificate-vm.ts` 헤더 · `LoadNodePageAllArgs.toViewModel` 문서).
        domains: toDomains(certificate.domains),
        // 제목과 「별칭」 행에 React 자식으로 나간다. 구 노드는 키가 없다 — `readAlias` 가 「없음」으로 읽는다.
        alias: readAlias(certificate.alias, "alias"),
        // `statusLabel` 의 「모르는 값은 그대로」 갈래가 React 자식이 된다 —
        // 그 함수는 렌더 중에 불려 관문 밖이라 여기서 읽는다.
        status: readText(certificate.status, "status") as CertificateStatus,
        source: certificate.source,
        staging: certificate.staging,
        challengeType: certificate.challengeType,
        dnsProvider: certificate.dnsProvider,
        issuedAt: readInstant(certificate.issuedAt),
        expiresAt: readInstant(certificate.expiresAt),
        lastAttemptAt: readInstant(certificate.lastAttemptAt),
        // 둘 다 화면이 **그대로 React 자식으로** 그린다(실패 횟수 / 빨간 배너의
        // `<pre>`). 옮겨 담기만 하면 원소 깊이가 관문 밖으로 샌다.
        failureCount: readNumber(certificate.failureCount, "failureCount"),
        lastError: readNullableText(certificate.lastError, "lastError"),
        issueLog: toIssueLogLines(certificate.issueLog),
        // **로컬 상세와 같은 값으로 가른다** — `dnsSubmittedAt` 이 있으면 검증
        // 중이다. `status` 로 짐작하면 갱신 주문(두 단계 다 `issued`)에서 틀린다
        // (`toNodeDnsOrder` 문서).
        //
        // **화면에 안 나가는데도 읽는다.** 이 값은 그려지는 게 아니라 **어느
        // 절반을 그릴지**를 고르고, 잘못 고르면 화면은 멀쩡한 채 사용자가 넣어야
        // 할 TXT 가 사라진다. 문자열이 아닌 무엇(예: `{}`)이 오면 truthy 라
        // 조용히 「검증 중」으로 접히므로, 여기서 읽어 §9 카드로 바꾼다 — 이
        // 계약 필드가 존재하는 이유가 바로 그 실패를 없애는 것이다.
        dnsOrder: toNodeDnsOrder(
          certificate.dnsRecords,
          readNullableText(certificate.dnsSubmittedAt, "dnsSubmittedAt"),
        ),
        // `toLinkedServer` 가 표의 두 셀(`serverName`·`listen`)을 원소 깊이로
        // 읽는다 — 로컬 상세와 같은 함수다.
        linkedServers: servers.filter((s) => s.certificateId === certificate.id).map(toLinkedServer),
      };
    },
  });

  if (!loaded.ok) return (
    <section className={layout.page}>
      <PageHeader title="인증서" />
      {loaded.render}
    </section>
  );

  // **`canWrite` 를 손으로 만들지 않는다** — `loadNodePageAll` 이 실어 준
  // `effectiveRole`(= min(사용자 등급, 노드 토큰 등급))로 판정한다
  // (Global Constraints).
  const canWrite = can(loaded.effectiveRole, "config:write");
  // 중단은 서버 액션이 아니라 이 중계 경로다(파일 헤더). **여기서 한 번 만들어
  // 두 컴포넌트에 같은 값을 넘긴다** — 한쪽만 로컬 경로로 두면 그 버튼이
  // 게이트웨이 자신의 발급을 끊으러 간다.
  const cancelPath = `/api/nodes/${encodeURIComponent(id)}/certificates/${encodeURIComponent(cid)}/cancel`;

  return (
    <CertificateDetail
      data={loaded.data}
      basePath={basePath}
      // **인증서 경로에서 파생하지 않는다** — 연결된 서버 행은 노드의 리버스
      // 프록시 상세로 가야 한다(`CertificateDetailProps.serverBasePath` 문서).
      serverBasePath={`/nodes/${id}/reverse-proxy`}
      // 교체 성공 뒤 노드 교체 화면이 붙여 보낸다 — 로컬과 같은 쿼리다.
      uncovered={uncovered}
      dnsPanel={
        // **`awaiting-txt` 에서만 그린다.** `verifying` 에 이 패널을 그리면 이미
        // 검증 중인 주문에 「TXT 를 넣으세요」라고 말하고(노드가 「이미 검증이
        // 진행 중입니다」로 거절한다), 반대로 `awaiting-txt` 에서 안 그리면
        // **사용자가 넣어야 할 TXT 를 감춘다.** 그 판정을 가능하게 한 것이
        // `dnsSubmittedAt` 이다(위 `toViewModel`).
        loaded.data.dnsOrder?.phase === "awaiting-txt" ? (
          <DnsChallengePanel
            id={cid}
            submitAction={{ id: "node.submitNodeDnsChallenge", args: [id] }}
            cancelPath={cancelPath}
            records={loaded.data.dnsOrder.records}
            mode={loaded.data.status === "awaiting-dns" ? "initial" : "renew"}
            canWrite={canWrite}
          />
        ) : null
      }
      actions={
        canWrite ? (
          <CertificateDetailActions
            targetName={loaded.data.alias ?? loaded.data.domains.primary ?? "인증서"}
            id={cid}
            cancelPath={cancelPath}
            editHref={`${basePath}/${cid}/edit`}
            // **노드 경로여야 한다.** 로컬 경로를 넘기면 노드의 수동 인증서를
            // 고치러 갔다가 게이트웨이 **자신의** 인증서를 열게 된다 — 같은 id
            // 가 양쪽에 있을 리 없어 대개 404 지만, 있으면 남의 인증서를 덮는다.
            replaceHref={`${basePath}/${cid}/replace`}
            status={loaded.data.status}
            linkedCount={loaded.data.linkedServers.length}
            source={loaded.data.source}
            challengeType={loaded.data.challengeType}
            dnsAutomated={loaded.data.dnsProvider === "cloudflare"}
            dnsOrderInFlight={!!loaded.data.dnsOrder}
            dnsVerifying={loaded.data.dnsOrder?.phase === "verifying"}
            retryAction={{ id: "node.retryNodeCertificate", args: [id] }}
            renewAction={{ id: "node.renewNodeCertificate", args: [id] }}
            renewDnsAction={{ id: "node.renewDnsNodeCertificate", args: [id] }}
            deleteAction={{ id: "node.deleteNodeCertificate", args: [id] }}
          />
        ) : null
      }
    />
  );
}
