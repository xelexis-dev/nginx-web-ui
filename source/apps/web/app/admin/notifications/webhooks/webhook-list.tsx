import { formatServerTime } from "@/lib/format-time";
import layout from "@/components/page-layout.module.css";
import {
  Icon,
  Facts,
  StatusBadge,
  EmptyState,
  Surface,
  buttonClassName,
  Table,
  Th,
  Td,
  Tr,
  PageHeader,
} from "@repo/ui";
import Link from "next/link";
import { eventLabel } from "@repo/presentation/webhook";
import type { WebhookRow } from "./form-types";
import styles from "../notifications.module.css";

// Admin-only. requireRole gates the route server-side (defense in depth alongside
// the admin layout's guard and each action's own guard).
// Reads live DB state on every request; must not be prerendered at build time.

const FORMAT_LABEL: Record<string, string> = {
  generic: "일반 (JSON)",
  discord: "Discord",
  dooray: "Dooray",
  custom: "일반 (JSON · 직접 작성)",
};

function lastResult(row: {
  lastAttemptAt: string | null;
  lastStatus: number | null;
  lastError: string | null;
}): { text: string; tone: "ok" | "error" | "none" } {
  if (!row.lastAttemptAt) return { text: "아직 보낸 적 없음", tone: "none" };
  const when = formatServerTime(new Date(row.lastAttemptAt));
  if (row.lastError) return { text: `실패 · ${when}`, tone: "error" };
  return { text: `성공 (HTTP ${row.lastStatus ?? "?"}) · ${when}`, tone: "ok" };
}

export function WebhookListView({ targets }: { targets: WebhookRow[] }) {
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>Webhook</>}
        info={
          <>인증서 알림을 Discord·사내 알림 서버 등 HTTP 수신처로 내보냅니다.</>
        }
        actions={
          <>
            {" "}
            <div className={styles.headerActions}>
              <Link
                href="/admin/notifications/webhooks/new"
                className={buttonClassName({ variant: "primary" })}
              >
                <Icon name="plus" />새 webhook
              </Link>
            </div>{" "}
          </>
        }
      />

      <Facts
        layout="strip"
        items={[
          { term: "수신처", value: targets.length },
          {
            term: "사용",
            value: targets.filter((item) => item.enabled).length,
          },
        ]}
      />

      <Surface title={<>수신처</>} as="section" flush>
        <Table label="Webhook 수신처">
          <thead>
            <Tr>
              <Th>이름</Th>
              <Th>형식</Th>
              <Th>사건</Th>
              <Th>상태</Th>
              {/* 실제 알림 발송만 기록한다 — 테스트 전송은 여기 남지 않는다(상세 참조). */}
              <Th>마지막 알림</Th>
            </Tr>
          </thead>
          <tbody>
            {targets.length === 0 ? (
              <Tr>
                <Td colSpan={5}>
                  <EmptyState
                    description={
                      <>
                        {" "}
                        등록된 webhook 이 없습니다. Discord 채널이나 사내 알림
                        서버를 추가해 보세요.{" "}
                      </>
                    }
                  />
                </Td>
              </Tr>
            ) : null}
            {targets.map((target) => {
              const events = (
                Array.isArray(target.events) ? target.events : []
              ) as string[];
              const result = lastResult(target);
              return (
                <Tr key={target.id}>
                  <Td cell="primary">
                    <Link href={`/admin/notifications/webhooks/${target.id}`}>
                      {target.name}
                    </Link>
                  </Td>
                  <Td cell="meta" label="형식">
                    {FORMAT_LABEL[target.format] ?? target.format}
                  </Td>
                  <Td
                    cell="meta"
                    label="사건"
                    title={events.map(eventLabel).join("\n")}
                  >
                    {events.length}건
                  </Td>
                  <Td cell="status">
                    <StatusBadge
                      tone={target.enabled ? "success" : "neutral"}
                      size="sm"
                    >
                      {target.enabled ? "사용" : "사용 안 함"}
                    </StatusBadge>
                  </Td>
                  <Td cell="meta" label="마지막 알림">
                    <StatusBadge
                      tone={result.tone === "error" ? "error" : "neutral"}

                      size="sm"
                    >
                      {result.text}
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
