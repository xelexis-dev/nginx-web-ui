import { formatServerTime } from "@/lib/format-time";
import type { GatewayView } from "@/lib/admin-api";
import { ApiForm } from "@/components/api-form";
import {
  EmptyState,
  Surface,
  Table,
  Th,
  Td,
  Tr,
  Button,
  StatusBadge,
  Banner,
} from "@repo/ui";
import styles from "./connected-gateways.module.css";

/**
 * 이 노드를 원격 제어할 수 있는 게이트웨이 목록.
 *
 * 해지된 것도 함께 보인다 — "누가 붙어 있나" 만큼 "누가 붙어 있었나" 도 보안
 * 기록이다(사용자 관리에서 비활성 계정을 감추지 않는 것과 같은 관용).
 */
export function ConnectedGateways({ rows }: { rows: GatewayView[] }) {
  return (
    <Surface
      title="연결된 게이트웨이"
      description="연결을 끊으면 그 게이트웨이는 즉시 이 서버를 읽지도 바꾸지도 못합니다. 다시 연결하려면 게이트웨이에서 이 서버를 다시 등록해야 합니다."
      info="이 서버를 원격으로 제어할 수 있는 게이트웨이입니다."
      flush
    >
      {rows.some((row) => row.legacyOrphaned === true && !row.revokedAt) ? (
        <div className={styles.notice}>
          <Banner
            tone="warning"
            title="현재 활성·승인된 관리자와 연결되지 않은 이전 연결입니다. 호환성을 위해 기존 접근을 유지합니다. 필요하지 않으면 연결을 끊으세요."
          />
        </div>
      ) : null}
      <Table label="연결된 게이트웨이">
        <thead>
          <Tr>
            <Th>이름</Th>
            <Th>권한</Th>
            <Th>연결된 때</Th>
            <Th>마지막 사용</Th>
            <Th>마지막 IP</Th>
            <Th>상태</Th>
            <Th />
          </Tr>
        </thead>
        <tbody>
          {rows.length === 0 ? (
            <Tr>
              <Td colSpan={7}>
                <EmptyState title="연결된 게이트웨이가 없습니다." />
              </Td>
            </Tr>
          ) : null}
          {rows.map((row) => (
            <Tr
              key={row.id}
              className={row.revokedAt ? styles.revoked : undefined}
            >
              <Td cell="primary">{row.name}</Td>
              <Td cell="meta" label="권한">
                {row.role}
              </Td>
              <Td cell="meta" label="연결된 때">
                {formatServerTime(new Date(row.createdAt))}
              </Td>
              <Td cell="meta" label="마지막 사용">
                {row.lastUsedAt
                  ? formatServerTime(new Date(row.lastUsedAt))
                  : "—"}
              </Td>
              <Td cell="meta" label="마지막 IP">
                {row.lastUsedIp ?? "—"}
              </Td>
              <Td cell="status" className={styles.statusCell}>
                <div className={styles.status}>
                  <span>
                    {row.revokedAt
                      ? `해지됨 (${formatServerTime(new Date(row.revokedAt))})`
                      : "연결됨"}
                  </span>
                  {row.legacyOrphaned === true ? (
                    <StatusBadge tone={row.revokedAt ? "neutral" : "warning"}>
                      이전 연결 · 발급자 확인 필요
                    </StatusBadge>
                  ) : null}
                </div>
              </Td>
              <Td cell="action">
                {row.revokedAt ? null : (
                  <ApiForm
                    action={{ id: "admin.security.revokeGateway", args: [] }}
                  >
                    <input type="hidden" name="id" value={row.id} />
                    <Button size="sm" type="submit" variant="secondary">
                      연결 끊기
                    </Button>
                  </ApiForm>
                )}
              </Td>
            </Tr>
          ))}
        </tbody>
      </Table>
    </Surface>
  );
}
