import { PageHeader } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { can } from "@/lib/permissions";
import { CertificateList } from "@/components/certificates/certificate-list";
import {
  readAlias,
  readInstant,
  readText,
  toDomains,
  type CertificateListVM,
  type CertificateSource,
  type CertificateStatus,
} from "@/components/certificates/certificate-vm";
import { loadNodePageAll } from "@/lib/node-page";

// 노드의 인증서 목록. 화면 본문은 components/certificates/certificate-list.tsx —
// 로컬 (app)/certificates/page.tsx 와 같은 컴포넌트를 쓴다.
//
// **엔드포인트를 둘 부른다.** 인증서 목록 + **서버 목록**(「연결 서버」 열).
// 로컬이 DB 를 두 번 읽는 것과 같은 이유다 — 서버 쪽을 게이트웨이 자기 것으로
// 채우면 남의 서버 수가 노드 인증서 옆에 붙는다.
//
// **인증서 응답만 원시 행이 아니다.** 다른 리소스는 노드가 테이블 행을 그대로
// 주는데(`api/v1/servers/route.ts`), 인증서는 `api/v1/certificates/view.ts` 의
// **허용목록**을 지나온다 — `certPem`/`keyPem`/DNS 토큰이 게이트웨이로 올라오지
// 않게 하려고 일부러 그렇게 돼 있다. 그래서 날짜가 `Date` 가 아니라 **ISO
// 문자열**이고, 아래에서 `readInstant` 로 되돌린다.
//
// **「가져오기」도 노드 경로다**(`${basePath}/import`). 이 자리는 한동안 `null`
// (= 버튼 없음)이었다 — PEM 등록 화면이 로컬 전용이던 시기다. `CertificateListWrite`
// 가 이 자리를 `boolean` 이 아니라 `string | null` 로 둔 덕에 그 시기에도 404 로
// 가는 버튼이 그려지지 않았고, 지금은 경로만 채우면 끝난다.
export const runtime = "nodejs";

/** `GET /api/v1/certificates` 의 응답 — `certificateView` 가 좁힌 항목들. */
interface NodeCertificatesResponse {
  certificates: {
    id: string;
    domains: string[];
    /** 별칭을 모르는 구 노드에는 키 자체가 없다 — 그래서 선택. */
    alias?: string | null;
    status: CertificateStatus;
    source: CertificateSource;
    staging: boolean;
    expiresAt: string | null;
  }[];
}

/** `GET /api/v1/servers` 의 응답. 노드는 `nginx_servers` 행을 그대로 준다. */
interface NodeServersResponse {
  servers: { certificateId: string | null }[];
}

export default async function NodeCertificatesPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const basePath = `/nodes/${id}/certificates`;

  const loaded = await loadNodePageAll<"certificates" | "servers", CertificateListVM>({
    nodeId: id,
    action: "config:read",
    paths: { certificates: "/api/v1/certificates", servers: "/api/v1/servers" },
    // 카드 하나가 조회 둘을 대표한다 — 이름이 없으면 사용자가 「인증서가
    // 사라졌다」로 읽는다(`LoadNodePageAllArgs.subjects` 문서).
    subjects: { certificates: "인증서 목록을", servers: "서버 목록을" },
    retryHref: basePath,
    // 목록 화면이라 둘이 같은 곳이다 — 같은 값을 그대로 넘긴다(우연이 아니라
    // 단언, `NodeErrorCardProps` 문서).
    listHref: basePath,
    // 응답 모양이 어긋나면 여기서 던지고 `loadNodePageAll` 이 `node_error` 카드로
    // 접는다 — 방어 코드를 두지 않는다(Global Constraints).
    toViewModel: (raw) => {
      const { certificates } = raw.certificates as NodeCertificatesResponse;
      const { servers } = raw.servers as NodeServersResponse;

      // **노드의 서버로 센다.** 게이트웨이 자기 목록으로 세면 「연결 서버 3」이
      // 뜨는데 노드에는 하나도 없는 상태가 된다.
      const linkedCount = new Map<string, number>();
      for (const s of servers) {
        if (s.certificateId) linkedCount.set(s.certificateId, (linkedCount.get(s.certificateId) ?? 0) + 1);
      }

      return {
        items: certificates.map((c) => ({
          id: c.id,
          // `toDomains`·`readInstant`·`readText` 는 **화면이 파고드는 깊이 그대로
          // 여기서 읽는다** — 통과만 시키면 그 실패가 렌더 도중에 터져 §9 카드가
          // 아니라 일반 500 화면이 된다(`certificate-vm.ts` 헤더).
          domains: toDomains(c.domains),
          // 화면에 React 자식으로 나간다. 구 노드는 키가 없다 — `readAlias` 가 그것을 「없음」으로 읽는다.
          alias: readAlias(c.alias, "alias"),
          // `statusLabel` 이 모르는 값을 **그대로 되돌려** React 자식이 되는데,
          // 그 함수는 렌더 중에 불려 관문 밖이다 — 그래서 여기서 읽는다.
          status: readText(c.status, "status") as CertificateStatus,
          source: c.source,
          staging: c.staging,
          expiresAt: readInstant(c.expiresAt),
          linkedCount: linkedCount.get(c.id) ?? 0,
        })),
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
  // `effectiveRole`(= min(사용자 등급, 노드 토큰 등급))로 판정한다. 세션에서 다시
  // 만들면 노드 토큰이 viewer 인데 버튼이 그려진다(Global Constraints).
  const canWrite = can(loaded.effectiveRole, "config:write");

  return (
    <CertificateList
      data={loaded.data}
      basePath={basePath}
      write={canWrite ? { newHref: `${basePath}/new`, importHref: `${basePath}/import` } : null}
    />
  );
}
