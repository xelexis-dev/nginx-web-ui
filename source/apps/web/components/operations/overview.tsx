import Link from "next/link";
import type { ReactNode } from "react";
import {
  Facts,
  Banner,
  KpiRow,
  linkClassName,
  EmptyState,
  Icon,
  PageHeader,
  StatusBadge,
  Surface,
  Table,
  Td,
  Th,
  Tr,
} from "@repo/ui";
import type { Optional } from "@/lib/optional";
import type { OverviewVM } from "@/lib/overview-data";
import styles from "./overview.module.css";
import layout from "@/components/page-layout.module.css";

/**
 * 개요(Operations). `lib/overview-data.ts` 가 이미 있는 뷰들로 합친 값만 그린다 — 지어낸 수치는
 * 없다. 보조 칸은 각자 실패한다: 읽지 못하면 그 칸에 이유가 뜨고, 권한이 없으면 그 칸이 사라진다.
 * 서버 상태는 **저장된 설정·인증서 기준**이다(서버 응답 확인이 아니다). 트래픽·p95·적용 실패 이력은
 * 출처가 없어 보여 주지 않는다 — 접속 지표 화면으로 보낸다.
 *
 * 순수 표시 컴포넌트다(읽기는 페이지의 `readOverview`). 그래서 Storybook 이 같은 컴포넌트를
 * 예시 값으로 그린다.
 */
export function OperationsOverview({ vm }: { vm: OverviewVM }) {
  const { counts, gauge } = vm;
  const metric = (v: number | undefined) =>
    v === undefined ? "—" : v.toLocaleString("ko-KR");

  return (
    <div className={layout.page}>
      <PageHeader
        title="개요"
        info={
          gauge
            ? "nginx 수치는 이 화면을 연 시각의 stub_status 스냅샷입니다."
            : undefined
        }
        description={
          !gauge ? "nginx 수치를 아직 수집하지 못했습니다." : undefined
        }
      />

      <div className={styles.sections}>
        <Surface
          variant="boxed"
          title="nginx 현황"
          actions={
            <Link
              href="/metrics"
              className={linkClassName({ variant: "back" })}
            >
              접속 지표 <Icon name="chevronRight" size={14} />
            </Link>
          }
        >
          <KpiRow
            ruled={false}
            label="nginx 실시간"
            columns={4}
            items={[
              {
                label: "활성 연결",
                icon: <Icon name="servers" />,
                value: metric(gauge?.active),
                note: "현재 연결된 클라이언트",
              },
              {
                label: "누적 요청",
                icon: <Icon name="metrics" />,
                value: metric(gauge?.requests),
                note: "nginx 시작 이후 처리한 요청",
              },
              {
                label: "대기(keep-alive)",
                icon: <Icon name="stream" />,
                value: metric(gauge?.waiting),
                note: (
                  <>
                    읽기 {metric(gauge?.reading)} · 쓰기{" "}
                    {metric(gauge?.writing)}
                  </>
                ),
              },
              {
                label: "드롭된 연결",
                icon: <Icon name="alert" />,
                value: gauge
                  ? metric(Math.max(0, gauge.accepts - gauge.handled))
                  : "—",
                note: "수락한 연결 중 처리되지 않은 연결",
              },
            ]}
          />
        </Surface>

        <div className={styles.overviewGrid}>
          <div className={styles.sectionStack}>
            <ServerHealth servers={vm.servers} />
            <EventsPanel events={vm.events} />
          </div>
          <div className={styles.sectionStack}>
            <AttentionPanel servers={vm.servers} />
            <PendingPanel servers={vm.servers} />
            <Surface
              variant="boxed"
              title="구성"
              footer={
                <>
                  <Link
                    href="/audit"
                    className={linkClassName({ variant: "back" })}
                  >
                    감사(24시간) {counts.audit24h.toLocaleString("ko-KR")}
                  </Link>
                  <span>
                    <Link
                      href="/history"
                      className={linkClassName({ variant: "back" })}
                    >
                      마지막 적용
                    </Link>{" "}
                    {lastApplyValue(vm.lastApply)}
                  </span>
                </>
              }
            >
              <Facts
                items={[
                  {
                    term: (
                      <Link
                        href="/reverse-proxy"
                        className={linkClassName({ variant: "back" })}
                      >
                        리버스 프록시
                      </Link>
                    ),
                    value: (
                      <>
                        {counts.serverActive} / {counts.serverTotal}{" "}
                        <span className={styles.small}>활성 / 전체</span>
                      </>
                    ),
                  },
                  {
                    term: (
                      <Link
                        href="/stream"
                        className={linkClassName({ variant: "back" })}
                      >
                        Stream
                      </Link>
                    ),
                    value: (
                      <>
                        {counts.streamActive} / {counts.streamTotal}{" "}
                        <span className={styles.small}>활성 / 전체</span>
                      </>
                    ),
                  },
                  {
                    term: (
                      <Link
                        href="/certificates"
                        className={linkClassName({ variant: "back" })}
                      >
                        인증서
                      </Link>
                    ),
                    value: (
                      <>
                        {vm.certificates.ok ? vm.certificates.data.total : "—"}{" "}
                        <span className={styles.small}>등록된 인증서</span>
                      </>
                    ),
                  },
                ]}
              />
              {vm.certificates.ok &&
              (vm.certificates.data.expiring || vm.certificates.data.failed) ? (
                <p className={styles.kpiNote}>
                  인증서 만료 임박(30일 이내·만료 포함){" "}
                  {vm.certificates.data.expiring}개 · 발급 실패{" "}
                  {vm.certificates.data.failed}개
                </p>
              ) : null}
            </Surface>
            <NodesPanel nodes={vm.nodes} />
          </div>
        </div>
      </div>
    </div>
  );
}

function lastApplyValue(value: OverviewVM["lastApply"]): ReactNode {
  if (!value.ok) return "—";
  if (!value.data) return "기록 없음";
  return <span className={styles.small}>{value.data.time}</span>;
}

/**
 * 보조 칸의 실패: 권한이 없으면 그리지 않고(`null`), 오류면 이유를 한 줄로 보인다. 면 안쪽 여백이 없는
 * (flush) 칸에서만 `inset` 으로 스스로 여백을 둔다 — 여백이 있는 칸에서 또 두면 제목보다 안으로 밀린다.
 */
function PanelFailure({
  value,
  inset = false,
}: {
  value: Extract<Optional<unknown>, { ok: false }>;
  inset?: boolean;
}) {
  if (value.reason === "forbidden") return null;
  const message = (
    <Banner tone="warning" title={<>읽지 못했습니다: {value.message}</>} />
  );
  return inset ? <div className={styles.inset}>{message}</div> : message;
}

function ServerHealth({ servers }: { servers: OverviewVM["servers"] }) {
  if (!servers.ok && servers.reason === "forbidden") return null;
  return (
    <Surface
      variant="boxed"
      title="서버 상태"
      info="저장된 설정·인증서 기준 · 확인이 필요한 서버가 위로"
      flush
      actions={
        servers.ok ? (
          <Link
            href="/reverse-proxy"
            className={linkClassName({ variant: "back" })}
          >
            전체 {servers.data.total}개
            <Icon name="chevronRight" size={14} />
          </Link>
        ) : undefined
      }
    >
      {!servers.ok ? (
        <PanelFailure value={servers} inset />
      ) : servers.data.rows.length === 0 ? (
        <div className={styles.emptyInset}>
          <EmptyState description="등록된 서버가 없습니다." />
        </div>
      ) : (
        <>
          {servers.data.certificatesUnknown ? (
            <div className={styles.inset}>
              <Banner
                tone="warning"
                title="인증서 상태를 읽지 못해 TLS 를 확인할 수 없습니다."
              />
            </div>
          ) : null}
          <Table label="서버 상태">
            <thead>
              <tr>
                <Th>상태</Th>
                <Th>서버</Th>
                <Th>대상</Th>
                <Th>TLS</Th>
              </tr>
            </thead>
            <tbody>
              {servers.data.rows.map((s) => (
                <Tr key={s.id} pending={s.pending}>
                  <Td cell="status">
                    <StatusBadge tone={s.health.tone}>
                      {s.health.label}
                    </StatusBadge>
                  </Td>
                  <Td
                    cell="primary"
                    wrap
                    icon={<Icon name="servers" />}
                    secondary={s.health.reason}
                  >
                    <span className={styles.name}>
                      <Link href={`/reverse-proxy/${s.id}`}>
                        {s.serverName.split(/\s+/)[0]}
                      </Link>
                      {s.pending ? (
                        <StatusBadge
                          tone="info"
                          variant="outline"
                          size="sm"
                          dot={false}
                        >
                          미적용
                        </StatusBadge>
                      ) : null}
                    </span>
                  </Td>
                  <Td mono wrap title={s.targetLabel}>
                    {s.targetLabel}
                  </Td>
                  <Td tone={s.tls.tone === "neutral" ? undefined : s.tls.tone}>
                    {s.tls.label}
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        </>
      )}
    </Surface>
  );
}

function AttentionPanel({ servers }: { servers: OverviewVM["servers"] }) {
  if (!servers.ok && servers.reason === "forbidden") return null;
  const rows = servers.ok
    ? servers.data.rows.filter(
        (s) => s.health.tone === "error" || s.health.tone === "warning",
      )
    : [];
  return (
    <Surface variant="boxed" title="확인 필요" info="저장된 설정·인증서 기준">
      {!servers.ok ? (
        <PanelFailure value={servers} />
      ) : (
        <div className={styles.stack}>
          <Facts
            layout="strip"
            items={[
              { term: "전체", value: `${servers.data.total}개` },
              { term: "확인 필요", value: `${servers.data.attention}개` },
            ]}
          />
          <ul className={styles.list}>
            {rows.map((s) => (
              <li key={s.id}>
                <StatusBadge tone={s.health.tone}>{s.health.label}</StatusBadge>
                <Link
                  href={`/reverse-proxy/${s.id}`}
                  className={linkClassName({ variant: "back" })}
                >
                  {s.serverName.split(/\s+/)[0]}
                </Link>
                {s.health.reason ? (
                  <span className={styles.muted}>{s.health.reason}</span>
                ) : null}
              </li>
            ))}
          </ul>
          {servers.data.attention > rows.length ? (
            <p className={styles.small}>
              외 {servers.data.attention - rows.length}건{" "}
              <Link
                href="/reverse-proxy"
                className={linkClassName({ variant: "back" })}
              >
                전체 보기
              </Link>
            </p>
          ) : null}
        </div>
      )}
    </Surface>
  );
}

const CHANGE_LABEL: Record<string, string> = {
  added: "추가",
  changed: "변경",
  removed: "삭제",
};
/** 옆 칸에 보이는 미적용 항목 수. 나머지는 개수로만 말한다(전체 diff 는 셸의 「설정 적용」에 있다). */
const PENDING_SHOWN = 6;

function PendingPanel({ servers }: { servers: OverviewVM["servers"] }) {
  if (!servers.ok && servers.reason === "forbidden") return null;
  const count = servers.ok ? servers.data.pendingCount : null;
  return (
    <Surface
      variant="boxed"
      title={count === null ? "미적용 변경" : `미적용 변경 ${count}건`}
      headingLevel={2}
    >
      {!servers.ok ? (
        <PanelFailure value={servers} />
      ) : !servers.data.pendingHasBaseline ? (
        <p className={styles.small}>
          적용 상태 확인 필요 — 현재 동작 중인 설정을 확인하지 못했습니다.
          설정을 확인한 뒤 다시 적용하세요.
        </p>
      ) : count === 0 ? (
        <p className={styles.small}>모든 변경이 nginx에 반영되어 있습니다.</p>
      ) : (
        <div className={styles.stack}>
          <ul className={styles.list}>
            {servers.data.pendingItems.slice(0, PENDING_SHOWN).map((item) => (
              <li key={`${item.status}-${item.key}`}>
                <span className={styles.strong}>{item.label}</span>
                <span className={styles.muted}>
                  {CHANGE_LABEL[item.status] ?? item.status}
                </span>
              </li>
            ))}
          </ul>
          {servers.data.pendingItems.length > PENDING_SHOWN ? (
            <p className={styles.small}>
              외 {servers.data.pendingItems.length - PENDING_SHOWN}건
            </p>
          ) : null}
          <p className={styles.small}>
            {servers.data.canApply
              ? "머리줄의 「설정 적용」으로 nginx에 반영합니다."
              : "적용 권한이 있는 사용자가 반영할 수 있습니다."}
          </p>
        </div>
      )}
    </Surface>
  );
}

function NodesPanel({ nodes }: { nodes: OverviewVM["nodes"] }) {
  if (!nodes.ok && nodes.reason === "forbidden") return null;
  return (
    <Surface
      variant="boxed"
      title="노드"
      actions={
        <Link href="/nodes" className={linkClassName({ variant: "back" })}>
          노드 관리
          <Icon name="chevronRight" size={14} />
        </Link>
      }
    >
      {!nodes.ok ? (
        <PanelFailure value={nodes} />
      ) : nodes.data.length === 0 ? (
        <p className={styles.small}>연결된 노드가 없습니다.</p>
      ) : (
        <p className={styles.small}>연결된 노드 {nodes.data.length}대</p>
      )}
    </Surface>
  );
}

function EventsPanel({ events }: { events: OverviewVM["events"] }) {
  if (!events.ok && events.reason === "forbidden") return null;
  return (
    <Surface
      variant="boxed"
      flush
      title="최근 이벤트"
      actions={
        <Link href="/audit" className={linkClassName({ variant: "back" })}>
          감사 로그
          <Icon name="chevronRight" size={14} />
        </Link>
      }
    >
      {!events.ok ? (
        <PanelFailure value={events} inset />
      ) : events.data.length === 0 ? (
        <div className={styles.inset}>
          <p className={styles.small}>기록된 이벤트가 없습니다.</p>
        </div>
      ) : (
        <ol className={styles.events}>
          {events.data.map((e) => (
            <li key={e.id}>
              <span className={styles.activityIcon} aria-hidden="true">
                <Icon name="audit" />
              </span>
              <time className={styles.time}>{e.time}</time>
              <span>
                <span className={styles.strong}>{e.action}</span>
                {e.target ? (
                  <span className={styles.muted}> {e.target}</span>
                ) : null}
                {e.actor ? (
                  <span className={styles.muted}> · {e.actor}</span>
                ) : null}
              </span>
            </li>
          ))}
        </ol>
      )}
    </Surface>
  );
}
