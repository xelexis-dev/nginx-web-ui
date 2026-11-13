import layout from "@/components/page-layout.module.css";
import {
  Surface,
  Facts,
  KpiRow,
  Icon,
  Banner,
  linkClassName,
  PageHeader,
  buttonClassName,
} from "@repo/ui";
import Link from "next/link";
import {
  NodeErrorCard,
  type NodeErrorCardProps,
} from "@/components/node-error";
import type { NodeView } from "@/lib/node-page";
import styles from "../../../(app)/nodes/nodes.module.css";

/**
 * `GET /api/v1/status` 응답 모양. `app/api/v1/status/route.ts:44-50` 이 정본이다 —
 * 필드를 지어내지 않는다.
 */
interface NodeStatusResponse {
  semver: string;
  counts: { servers: number; streams: number; upstreams: number };
  pending: { count: number; hasBaseline: boolean };
  certificates: { total: number; expiringSoon: number };
  nginx: { configOk: boolean; output: string };
}

export type NodeOverviewView = { node: NodeView; canManage: boolean } & (
  | { ok: true; data: NodeStatusResponse }
  | { ok: false; nodeError: NodeErrorCardProps }
);

/** No polling: this displays the nginx -t result read once by the server route. */
export function NodeOverview({
  id,
  result,
}: {
  id: string;
  result: NodeOverviewView;
}) {
  const { node, canManage: canWrite } = result;
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>{node.name}</>}
        eyebrow={
          <>
            {" "}
            <Link href="/nodes" className={linkClassName({ variant: "back" })}>
              <Icon name="arrowLeft" size={14} />
              목록으로
            </Link>{" "}
          </>
        }
        actions={
          <>
            {" "}
            {canWrite ? (
              <div className={styles.headerActions}>
                <Link href={`/nodes/${id}/edit`} className={buttonClassName({ variant: "secondary" })}>
                  수정
                </Link>
              </div>
            ) : null}{" "}
          </>
        }
        description={<>{node.baseUrl}</>}
      />

      {!result.ok ? (
        <NodeErrorCard {...result.nodeError} />
      ) : (
        <>
          <KpiRow label="노드 요약" columns={4} items={[
            { label: "Reverse Proxy 서버", value: result.data.counts.servers, icon: <Icon name="route" /> },
            { label: "Stream", value: result.data.counts.streams, icon: <Icon name="cable" /> },
            { label: "Upstream", value: result.data.counts.upstreams, icon: <Icon name="upstreams" /> },
            { label: "인증서", value: result.data.certificates.total, icon: <Icon name="shieldCheck" /> },
          ]} />
          <Surface as="div"><Facts layout="strip" items={[
            { term: "버전", value: `v${result.data.semver}` },
            { term: "30일 내 만료", value: `${result.data.certificates.expiringSoon}개` },
            { term: "미적용 변경", value: <>
              {!result.data.pending.hasBaseline ? "적용 상태 확인 필요" : result.data.pending.count === 0 ? "없음" : `${result.data.pending.count}건`}
              {!result.data.pending.hasBaseline ? <span className={styles.hint}> — 현재 동작 중인 설정을 확인하지 못했습니다. 설정을 확인한 뒤 다시 적용하세요.</span> : null}
            </> },
          ]} /></Surface>

          {/* nginx.configOk 는 nginx -t 결과이지 프로세스 생존이 아니다 — 문구를
              "nginx 실행 중"이 아니라 "설정 유효"로 쓴다. */}
          <Banner
            tone={result.data.nginx.configOk ? "success" : "error"}
            title={
              <>
                {result.data.nginx.configOk
                  ? "nginx 설정 유효"
                  : "nginx 설정 오류 — nginx -t 실패"}
              </>
            }
            role="status"
            output={result.data.nginx.output}
          ></Banner>
        </>
      )}
    </section>
  );
}
