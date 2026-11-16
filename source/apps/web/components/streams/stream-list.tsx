import {
  Banner,
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
} from "@repo/ui";
import Link from "next/link";
import {
  findReservedPortConflict,
  type ReservedPorts,
} from "@repo/presentation/nginx/reserved-ports";
import layout from "@/components/page-layout.module.css";
import styles from "./stream.module.css";

/**
 * Stream 목록 화면의 본문. 로컬 화면(`app/(app)/stream/page.tsx`)과 노드 화면
 * (`app/(node)/nodes/[id]/stream/page.tsx`)이 같은 것을 렌더한다 — 데이터
 * 출처와 basePath만 다르다. 설정 적용은 각 화면의 공통 헤더가 담당한다.
 *
 * **이 디렉터리의 CSS 는 「옮긴」 쪽이다** — `(app)/stream/stream.module.css` 를
 * 통째로 여기로 옮겼고 `(app)/stream/` 에는 모듈이 남지 않았다. 왜 옮길 수
 * 있었는지는 `stream.module.css` 헤더에 있다(Upstream 은 못 옮겨서 라우트 그룹
 * 시트를 가져다 쓴 반대 경우다).
 *
 * **불변식: 공유 컴포넌트는 라우트 그룹의 CSS 모듈은 가져다 써도 TS/TSX 는 절대
 * 가져오지 않는다**(`components/upstreams/upstream-list.tsx` 헤더). 여기는 CSS
 * 조차 자기 것이라 그 문제가 아예 없다.
 *
 * **`basePath` 하나로 화면 안의 모든 stream 링크가 결정된다.** 노드 경로가
 * `/nodes/<id>/stream`(로컬 `/stream` 과 같은 꼬리)여야 하는 이유가 이것이다.
 */

export interface StreamRowVM {
  id: string;
  listen: number;
  protocol: "tcp" | "udp";
  proxyPass: string;
  enabled: boolean;
}

export interface StreamListVM {
  items: StreamRowVM[];
  /** 「+ 새 Stream」을 보여줄지. 로컬은 `can(session.user.role, …)`, 노드는 `can(effectiveRole, …)`. */
  canWrite: boolean;
}

export interface StreamListProps {
  data: StreamListVM;
  /** `/stream`(로컬) 또는 `/nodes/<id>/stream`(노드). */
  basePath: string;
  /**
   * 이 인스턴스가 이미 듣고 있는 포트. 넘기면 **거기 걸린 행에 경고를 붙인다.**
   *
   * 왜 필요한가: 부팅 가드(`docker/s6-rc.d/nginx/run`)가 예약 포트를 잡은 stream
   * 파일을 격리해 인스턴스를 살리는데, 그 사실이 **`docker logs` 에만** 남으면
   * 운영자는 목록에서 멀쩡해 보이는 행을 보게 된다 — 셸로 들어가야 알 수 있다는
   * 뜻이고, 그건 이 제품이 없애려는 바로 그 작업이다.
   *
   * **노드 화면은 넘기지 않는다** — 그 값은 노드의 env·DB 에서 나오는 것이라
   * 게이트웨이가 알 수 없고, 자기 값으로 판정하면 남의 인스턴스 행에 **틀린 경고**를
   * 붙이게 된다(폼의 `reservedListenPorts` 와 같은 판단).
   */
  reservedPorts?: ReservedPorts;
}

export function StreamList({ data, basePath, reservedPorts }: StreamListProps) {
  const { items, canWrite } = data;

  const conflictOf = (s: StreamRowVM): string | null => {
    if (!reservedPorts) return null;
    return (
      findReservedPortConflict(
        s.listen,
        s.protocol === "udp" ? "stream-udp" : "stream-tcp",
        reservedPorts,
      )?.message ?? null
    );
  };
  const conflicted = items.filter((s) => conflictOf(s) !== null).length;

  return (
    <section className={layout.page}>
      <PageHeader
        title="Stream"
        info="L4 (TCP/UDP) stream 서버를 관리하고, 변경 사항을 nginx에 적용합니다."
        actions={
          canWrite ? (
            <Link href={`${basePath}/new`} className={buttonClassName()}>
              <Icon name="plus" />새 Stream
            </Link>
          ) : null
        }
      />

      <Facts
        layout="strip"
        items={[
          { term: "Stream", value: items.length },
          { term: "활성", value: items.filter((item) => item.enabled).length },
          {
            term: "TCP",
            value: items.filter((item) => item.protocol === "tcp").length,
          },
          {
            term: "UDP",
            value: items.filter((item) => item.protocol === "udp").length,
          },
        ]}
      />

      {conflicted > 0 ? (
        <Banner
          tone="warning"
          role="status"
          title={`${conflicted}건의 stream 이 이 인스턴스가 쓰거나 다른 항목이 이미 등록한 포트를 잡고 있습니다.`}
        >
          {/* 이 배너는 **부팅 가드가 무엇을 했는지 단정하지 않는다.** 예약 집합은
              `enabled` 를 보지 않고 모으는데(껐다 켜는 경로로 구멍이 돌아오지 않게 —
              `lib/reserved-ports.ts`), 부팅 가드는 **렌더된 파일만** 본다
              (`docker/s6-rc.d/nginx/run`). 그래서 「꺼진 리버스 프록시가 :8443 +
              켜진 stream 이 :8443」이면 가드는 격리하지 않고 그 stream 은 **정상
              동작한다.** 「nginx 가 건너뛰었으므로 동작하지 않습니다」로 쓰면 그
              경우에 화면이 거짓을 말한다. 확실한 것은 조합이 위험하다는 것뿐이다. */}
          그 항목이 함께 켜지는 순간 nginx 가 시작하지 못하므로, 부팅할 때 해당
          stream 이 자동으로 제외될 수 있습니다 — 아래 표시된 행의 포트를
          바꾸거나 지운 뒤 설정을 다시 적용하세요.
        </Banner>
      ) : null}

      <Surface title="Stream" flush>
        <Table density="comfortable" narrow="stacked" label="Stream">
          <thead>
            <tr>
              <Th>listen</Th>
              <Th>protocol</Th>
              <Th>proxy_pass</Th>
              <Th>상태</Th>
            </tr>
          </thead>
          <tbody>
            {items.length === 0 ? (
              <Tr>
                <Td colSpan={4}>
                  <EmptyState title="등록된 stream이 없습니다" />
                </Td>
              </Tr>
            ) : null}
            {items.map((s) => {
              const conflict = conflictOf(s);
              return (
                <Tr key={s.id}>
                  <Td cell="primary" icon={<Icon name="cable" size={16} />}>
                    <Link href={`${basePath}/${s.id}`}>{s.listen}</Link>
                    {conflict ? (
                      <p className={styles.rowWarning}>{conflict}</p>
                    ) : null}
                  </Td>
                  <Td label="protocol">{s.protocol}</Td>
                  <Td mono wrap label="proxy_pass">
                    {s.proxyPass}
                  </Td>
                  <Td cell="status" label="상태">
                    <StatusBadge tone={s.enabled ? "success" : "neutral"}>
                      {s.enabled ? "활성" : "비활성"}
                    </StatusBadge>
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
