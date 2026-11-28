import {
  Facts,
  EmptyState,
  Icon,
  PageHeader,
  StatusBadge,
  Surface,
  Table,
  Td,
  Th,
  Tr,
  buttonClassName,
  type Tone,
} from "@repo/ui";
import Link from "next/link";
import {
  daysLeft,
  statusLabel,
  type CertificateListVM,
} from "./certificate-vm";
import layout from "@/components/page-layout.module.css";
import styles from "@/app/(app)/certificates/certificates.module.css";

/**
 * 인증서 목록 화면의 본문. 로컬(`app/(app)/certificates/page.tsx`)과
 * 노드(`app/(node)/nodes/[id]/certificates/page.tsx`)가 같은 것을 렌더한다.
 * 다른 것은 데이터 출처(로컬 DB vs `loadNodePageAll`)와 쓰기 링크뿐이다.
 *
 * **CSS 는 「가져다 쓰는」 쪽이다**(`components/reverse-proxy/*` 와 같은 판정).
 * `certificates.module.css` 는 `composes` 를 쓰고(`composes` 는 한 모듈 안에서만
 * 풀린다), 그 클래스들을 로컬에 남는 형제 화면들(`certificate-form.tsx`,
 * `import/page.tsx`, `[id]/replace/page.tsx`, `[id]/dns-challenge-panel.tsx` …)이
 * 함께 쓴다 — 그래서 옮길 수 없다. 그 사실은 원본 모듈 헤더의 소비자 목록에도
 * 적혀 있다.
 *
 * **불변식: 공유 컴포넌트는 라우트 그룹의 CSS 모듈은 가져다 써도 TS/TSX 는 절대
 * 가져오지 않는다**(`components/upstreams/upstream-list.tsx` 헤더). 상태 표기·
 * 도메인 요약이 `certificate-vm.ts` 로 간 것이 그래서다.
 *
 * **`basePath` 하나로 화면 안의 행 링크가 결정된다** — 로컬 `/certificates`,
 * 노드 `/nodes/<id>/certificates`. 꼬리가 같아야 치환이 한 번으로 끝나고
 * 사이드바 강조(`(app)/nav-links.ts` 의 `activeHref`, 최장 접두사)도 로컬과
 * 같게 동작한다.
 */

/**
 * 쓰기 UI 한 덩어리. **`canWrite: boolean` 하나로 대신하지 않는다** —
 * `ServerListWrite` 와 같은 이유다: 「그릴까 말까」와 「어디로 보낼까」가
 * 갈라지면 아직 없는 경로에도 버튼이 그려지고 아무 검사도 그걸 못 잡는다.
 */
export interface CertificateListWrite {
  /** 「+ 새 인증서(발급)」가 갈 곳. */
  newHref: string;
  /**
   * 「인증서 가져오기」(수동 PEM 등록)가 갈 곳. `null` 이면 버튼이 사라진다.
   *
   * **`null` 갈래를 지우지 마라.** 지금은 로컬·노드 둘 다 화면이 있어 언제나
   * 경로가 채워지지만, 이 자리를 `boolean` 으로 접었다면 화면이 없던 시기에
   * **404 로 가는 버튼**이 그려졌을 것이다(`ServerListWrite` 와 같은 규칙:
   * 「그릴까 말까」와 「어디로 보낼까」를 가르지 않는다).
   */
  importHref: string | null;
}

export interface CertificateListProps {
  data: CertificateListVM;
  /** `/certificates`(로컬) 또는 `/nodes/<id>/certificates`(노드). */
  basePath: string;
  /**
   * 쓰기 UI. **등급이 모자라거나(viewer) 아직 쓰기 경로가 없는 화면이면 `null`**
   * 이고 그때는 두 버튼 다 그리지 않는다.
   */
  write: CertificateListWrite | null;
}

/** 상태 배지의 색. 문구는 `statusLabel`(뷰모델)이고 색은 여기다 — 색은 CSS 모듈에 매인다. */
export function statusTone(status: string): Tone {
  if (status === "issued") return "success";
  if (status === "issuing") return "info";
  if (status === "awaiting-dns") return "warning";
  return "error";
}

export function CertificateList({
  data,
  basePath,
  write,
}: CertificateListProps) {
  return (
    <section className={layout.page}>
      <PageHeader
        title="인증서"
        info="ACME(Let's Encrypt) 인증서를 발급·갱신하고 reverse proxy 서버에 연결합니다."
        actions={
          write ? (
            <>
              {write.importHref ? (
                <Link
                  href={write.importHref}
                  className={buttonClassName({ variant: "secondary" })}
                >
                  인증서 가져오기
                </Link>
              ) : null}
              <Link href={write.newHref} className={buttonClassName()}>
                <Icon name="plus" />새 인증서(발급)
              </Link>
            </>
          ) : null
        }
      />

      <Facts
        layout="strip"
        items={[
          { term: "인증서", value: data.items.length },
          {
            term: "발급 완료",
            value: data.items.filter((item) => item.status === "issued").length,
          },
          {
            term: "수동",
            value: data.items.filter((item) => item.source === "manual").length,
          },
          {
            term: "연결 서버",
            value: data.items.reduce((sum, item) => sum + item.linkedCount, 0),
          },
        ]}
      />

      <Surface title="인증서" flush>
        <Table density="comfortable" narrow="stacked" label="인증서">
          <thead>
            <tr>
              <Th>도메인</Th>
              <Th>상태</Th>
              <Th>만료</Th>
              <Th align="end">연결 서버</Th>
            </tr>
          </thead>
          <tbody>
            {data.items.length === 0 ? (
              <Tr>
                <Td colSpan={4}>
                  <EmptyState description="발급된 인증서가 없습니다." />
                </Td>
              </Tr>
            ) : null}
            {data.items.map((c) => {
              const left = daysLeft(c.expiresAt);
              return (
                <Tr key={c.id}>
                  <Td cell="primary" icon={<Icon name="certificate" />}>
                    {/* 별칭이 있으면 그것이 링크 이름이고 도메인은 아래 줄로 — 상세 제목과 같은 배치. */}
                    <Link href={`${basePath}/${c.id}`}>
                      {c.alias ?? c.domains.primary ?? "—"}
                      {!c.alias && c.domains.extra > 0
                        ? ` 외 ${c.domains.extra}`
                        : ""}
                    </Link>
                    {c.alias ? (
                      <span className={styles.rowSub}>
                        {c.domains.primary ?? "—"}
                        {c.domains.extra > 0 ? ` 외 ${c.domains.extra}` : ""}
                      </span>
                    ) : null}
                  </Td>
                  <Td cell="status" label="상태">
                    <span className={styles.badgeRow}>
                      <StatusBadge tone={statusTone(c.status)}>
                        {statusLabel(c.status)}
                      </StatusBadge>
                      {c.source === "manual" ? (
                        <StatusBadge tone="neutral" dot={false}>
                          수동
                        </StatusBadge>
                      ) : null}
                      {c.staging ? (
                        <StatusBadge tone="warning" dot={false}>
                          STAGING
                        </StatusBadge>
                      ) : null}
                    </span>
                  </Td>
                  <Td
                    label="만료"
                    tone={
                      left !== null && left >= 0 && left < 30
                        ? "warning"
                        : undefined
                    }
                  >
                    {left === null ? (
                      "—"
                    ) : left < 0 ? (
                      <StatusBadge tone="error">만료</StatusBadge>
                    ) : (
                      `D-${left}`
                    )}
                  </Td>
                  <Td numeric label="연결 서버">
                    {c.linkedCount}
                  </Td>
                </Tr>
              );
            })}
          </tbody>
        </Table>
      </Surface>
    </section>
  );
}
