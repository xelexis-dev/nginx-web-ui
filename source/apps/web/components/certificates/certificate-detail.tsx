import {
  linkClassName,
  Banner,
  EmptyState,
  Facts,
  Icon,
  PageHeader,
  Surface,
  StatusBadge,
  Table,
  Td,
  Th,
  Tr,
} from "@repo/ui";
import type { ReactNode } from "react";
import Link from "next/link";
import { IssuancePoller } from "./issuance-poller";
import { statusTone } from "./certificate-list";
import { IssueLogPanel } from "./issue-log-panel";
import {
  fmtInstant,
  isLiveIssuance,
  statusLabel,
  type CertificateDetailVM,
} from "./certificate-vm";
import layout from "@/components/page-layout.module.css";
import styles from "@/app/(app)/certificates/certificates.module.css";

/**
 * 인증서 상세 화면의 본문. 로컬(`app/(app)/certificates/[id]/page.tsx`)과
 * 노드(`app/(node)/nodes/[id]/certificates/[cid]/page.tsx`)가 같은 것을 렌더한다.
 *
 * 상태·만료·마지막 시도 + 실패 시 `lastError` 배너(요청사항 — "발급 실패 오류
 * 메세지도 웹에서 보여야 함") + 발급 로그 + 연결 서버 목록. 발급/검증이 도는
 * 동안 `IssuancePoller` 가 2초 간격으로 새로고침한다.
 *
 * **이 파일은 서버 컴포넌트로 남는다**(`"use client"` 없음) — 추출 전 로컬
 * 상세가 RSC 였고, 클라이언트 훅이 필요한 것은 액션 버튼들과 DNS 패널뿐이다.
 * 그 둘은 아래 슬롯으로 내려온다(`components/reverse-proxy/server-detail.tsx` 가
 * 토글·삭제를 prop 으로 받는 것과 같은 갈래).
 *
 * **CSS 는 「가져다 쓰는」 쪽이다** — 이유는 `certificate-list.tsx` 헤더.
 */

export interface CertificateDetailProps {
  data: CertificateDetailVM;
  /** `/certificates`(로컬) 또는 `/nodes/<id>/certificates`(노드) — 「← 인증서 목록」. */
  basePath: string;
  /**
   * 연결된 서버 행이 갈 곳의 앞부분 — `/reverse-proxy`(로컬) 또는
   * `/nodes/<id>/reverse-proxy`(노드).
   *
   * **`basePath` 에서 파생하지 않는다.** 인증서 경로와 서버 경로는 꼬리가 달라
   * 문자열을 주무르면 노드에서 게이트웨이 자기 서버로 새어나간다 — 그 링크는
   * 열어 보기 전에는 멀쩡해 보인다.
   */
  serverBasePath: string;
  /**
   * 교체(replace) 성공 후 「새 인증서가 커버하지 않는 이름」 경고. 비어 있으면
   * 배너가 없다. 로컬·노드 둘 다 교체 액션이 `?uncovered=` 쿼리로 넘겨 준다.
   */
  uncovered: string[];
  /**
   * DNS-01 TXT 안내 패널이 들어갈 자리. **그릴지 말지는 호출부가 정한다**
   * (`data.dnsOrder?.phase === "awaiting-txt"`) — 패널이 클라이언트 컴포넌트라
   * 어느 서버 액션에 붙는지를 라우트 그룹마다 스스로 정해야 하기 때문이다.
   * 자리(발급 로그 다음, 연결 서버 앞)는 여기가 정한다.
   */
  dnsPanel: ReactNode;
  /**
   * 쓰기 액션 행(다시 발급 / 지금 갱신 / 중단 / 삭제 / 설정 수정)이 들어갈 자리.
   * 등급이 모자라거나 그 화면에 아직 쓰기 경로가 없으면 `null` 이다.
   *
   * **`dnsPanel` 과 함께 optional 로 두지 않았다** — 빠뜨린 화면이 조용히
   * 읽기 전용이 되는데, 그건 실패했을 때만 드러나는 자리다(`listHref` 를
   * 필수로 만든 것과 같은 계열).
   */
  actions: ReactNode;
}

export function CertificateDetail({
  data,
  basePath,
  serverBasePath,
  uncovered,
  dnsPanel,
  actions,
}: CertificateDetailProps) {
  const live = isLiveIssuance(data);
  const phase = data.dnsOrder?.phase ?? null;

  return (
    <section className={layout.page}>
      {live ? <IssuancePoller /> : null}
      <PageHeader
        title={
          <>
            {/* 별칭이 있으면 그것이 이름이고 대표 도메인은 옆에 작게 남는다 — 서버 폼 Select 라벨과 같은 두 값. */}
            {data.alias ? (
              <>
                {data.alias}{" "}
                <span className={styles.titleSub}>
                  {data.domains.primary ?? ""}
                </span>
              </>
            ) : (
              (data.domains.primary ?? "인증서")
            )}
          </>
        }
        description={
          <StatusBadge
            tone={statusTone(data.status)}
          >{`${statusLabel(data.status)}${data.staging ? " · STAGING" : ""}${phase === "verifying" ? " · TXT 확인·검증 중" : ""}${live ? " (자동 새로고침 중...)" : ""}`}</StatusBadge>
        }
        actions={
          actions ? <div className={styles.headerActions}>{actions}</div> : null
        }
        eyebrow={
          <Link href={basePath} className={linkClassName({ variant: "back" })}>
            <Icon name="back" size={14} />
            인증서 목록
          </Link>
        }
      />

      {uncovered.length > 0 ? (
        <Banner tone="warning" title="교체 완료 — 커버리지 경고">
          새 인증서가 연결 서버의 다음 이름을 커버하지 않습니다:{" "}
          {uncovered.join(", ")} — 해당 이름으로 접속하면 브라우저 인증서 경고가
          발생합니다.
        </Banner>
      ) : null}

      <div className={layout.detail}>
        <div className={layout.section}>
          <Surface title="인증서 정보">
            <Facts
              columns={2}
              items={[
                ...(data.alias ? [{ term: "별칭", value: data.alias }] : []),
                { term: "도메인", value: data.domains.all, mono: true },
                {
                  term: "종류",
                  value:
                    data.source === "manual"
                      ? "수동 등록"
                      : `Let's Encrypt (ACME · ${
                          data.challengeType === "dns-01"
                            ? data.dnsProvider === "cloudflare"
                              ? "DNS-01 자동 · Cloudflare"
                              : "DNS-01 수동 TXT"
                            : "HTTP-01"
                        })`,
                },

                {
                  term: "발급일",
                  value: fmtInstant(data.issuedAt),
                  mono: true,
                },
                {
                  term: "만료일",
                  value: fmtInstant(data.expiresAt),
                  mono: true,
                },
                ...(data.source === "acme"
                  ? [
                      {
                        term: "마지막 시도",
                        value: fmtInstant(data.lastAttemptAt),
                        mono: true,
                      },
                      {
                        term: "실패 횟수",
                        value: String(data.failureCount),
                        mono: true,
                      },
                    ]
                  : []),
              ]}
            />
          </Surface>

          {data.source === "acme" && data.lastError ? (
            <Banner
              tone="error"
              title="마지막 발급/갱신 실패"
              output={data.lastError}
            />
          ) : null}

          {data.source === "acme" && (data.issueLog.length > 0 || live) ? (
            <IssueLogPanel entries={data.issueLog} live={live} />
          ) : null}

          {dnsPanel}
        </div>
        <Surface title={`연결된 서버 (${data.linkedServers.length})`} flush>
          {data.linkedServers.length === 0 ? (
            <EmptyState
              title="연결된 서버가 없습니다"
              description="서버 편집 페이지의 TLS 섹션에서 연결하세요."
            />
          ) : (
            <Table label="연결된 서버">
              <thead>
                <tr>
                  <Th>server_name</Th>
                  <Th align="end">listen</Th>
                </tr>
              </thead>
              <tbody>
                {data.linkedServers.map((s) => (
                  <Tr key={s.id}>
                    <Td cell="primary">
                      <Link href={`${serverBasePath}/${s.id}`}>
                        {s.serverName}
                      </Link>
                    </Td>
                    <Td numeric label="listen">
                      {s.listen}
                    </Td>
                  </Tr>
                ))}
              </tbody>
            </Table>
          )}
        </Surface>
      </div>
    </section>
  );
}
