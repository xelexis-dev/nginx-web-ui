import layout from "@/components/page-layout.module.css";
import {
  Icon,
  Banner,
  Surface,
  Button,
  buttonClassName,
  Table,
  Th,
  Td,
  Tr,
  EmptyState,
  PageHeader,
} from "@repo/ui";
import Link from "next/link";
import type { NodeView } from "@/lib/node-page";
import { ApiForm } from "@/components/api-form";
import { NodeStatusBadge } from "@/components/node-error";
import styles from "./nodes.module.css";

const RELATIVE_UNITS: {
  limitSec: number;
  divisor: number;
  unit: Intl.RelativeTimeFormatUnit;
}[] = [
  { limitSec: 60, divisor: 1, unit: "second" },
  { limitSec: 3_600, divisor: 60, unit: "minute" },
  { limitSec: 86_400, divisor: 3_600, unit: "hour" },
  { limitSec: 2_592_000, divisor: 86_400, unit: "day" },
  { limitSec: 31_536_000, divisor: 2_592_000, unit: "month" },
];

/** "3분 전" 류의 상대 시각. Node 배포판은 full-ICU 를 내장해 "ko" 로케일이 그대로 된다. */
function formatRelative(date: Date): string {
  const diffSec = Math.max(0, Math.round((Date.now() - date.getTime()) / 1000));
  const rtf = new Intl.RelativeTimeFormat("ko", { numeric: "auto" });
  for (const { limitSec, divisor, unit } of RELATIVE_UNITS) {
    if (diffSec < limitSec)
      return rtf.format(-Math.round(diffSec / divisor), unit);
  }
  return rtf.format(-Math.round(diffSec / 31_536_000), "year");
}

/**
 * `lastSeenAt` 은 **성공했을 때만** 갱신된다(`recordProbe` 의 규칙) — 그래서
 * `lastStatus === null` (한 번도 관측되지 않음)과 `lastSeenAt === null` (관측은
 * 됐지만 한 번도 성공하지 못함)은 다른 뜻이고, 문구도 갈라야 한다.
 */
function lastSeenLabel(node: {
  lastStatus: string | null;
  lastSeenAt: string | null;
}): string {
  if (node.lastStatus === null)
    return "아직 확인되지 않았습니다 — 새로고침하세요.";
  if (node.lastSeenAt)
    return `${formatRelative(new Date(node.lastSeenAt))} 응답함`;
  return "응답한 적 없음";
}

export interface NodeListView {
  nodes: NodeView[];
  canWrite: boolean;
  unpairWarningText: string | null;
}

export function NodeList({
  nodes: nodeRows,
  canWrite,
  unpairWarningText,
}: NodeListView) {
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>노드</>}
        info={<>연결된 nginx-web-ui 노드와 마지막으로 확인된 상태를 봅니다.</>}
        actions={
          <>
            {" "}
            <div className={styles.listActions}>
              {canWrite ? (
                <ApiForm action={{ id: "nodes.refreshStatuses", args: [] }}>
                  <Button type="submit" variant="secondary">
                    상태 새로고침
                  </Button>
                </ApiForm>
              ) : null}
              {canWrite ? (
                <Link
                  href="/nodes/new"
                  className={buttonClassName({ variant: "primary" })}
                >
                  <Icon name="plus" />
                  노드 연결
                </Link>
              ) : null}
            </div>{" "}
          </>
        }
      />

      {unpairWarningText ? (
        <Banner
          tone={"warning"}
          title={<>{unpairWarningText}</>}
          role="status"
        ></Banner>
      ) : null}

      <Surface flush>
        <Table label="노드" density="comfortable" narrow="stacked">
          <thead>
            <tr>
              <Th>이름</Th>
              <Th>주소</Th>
              <Th>상태</Th>
              <Th>마지막 확인</Th>
            </tr>
          </thead>
          <tbody>
            {nodeRows.length === 0 ? (
              <Tr>
                <Td colSpan={4}>
                  <EmptyState description="연결된 노드가 없습니다." />
                </Td>
              </Tr>
            ) : null}
            {nodeRows.map((node) => (
              <Tr key={node.id}>
                <Td cell="primary" wrap icon={<Icon name="server" size={16} />}>
                  <Link className={styles.nodeTitle} href={`/nodes/${node.id}`}>
                    {node.name}
                  </Link>
                </Td>
                <Td mono wrap label="주소">
                  {node.baseUrl}
                </Td>
                <Td cell="status" label="상태">
                  <NodeStatusBadge
                    status={node.lastStatus}
                    version={node.lastVersion}
                  />
                </Td>
                <Td wrap label="마지막 확인">
                  {lastSeenLabel(node)}
                </Td>
              </Tr>
            ))}
          </tbody>
        </Table>
      </Surface>
    </section>
  );
}
