import { formatServerTime } from "@/lib/format-time";
import layout from "@/components/page-layout.module.css";
import {
  Icon,
  Facts,
  StatusBadge,
  Banner,
  linkClassName,
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
import type { MailRecipientRow, SmtpView } from "@repo/contracts";
import { eventLabel } from "@repo/presentation/webhook";

import styles from "../notifications.module.css";

// Reads live API state on every request; must not be prerendered at build time.

/**
 * 메일 **알림 채널** — 무엇을 누구에게. 보내는 수단(서버·자격증명)은 사이트 설정의
 * SMTP 설정에 있다. 이 둘을 한 화면에 두면 "SMTP 를 켰는데 왜 안 오지"와 "주소를
 * 넣었는데 왜 안 오지"가 뒤섞인다.
 */
export function RecipientListView({
  recipients,
  mailer,
}: {
  recipients: MailRecipientRow[];
  mailer: SmtpView["readiness"];
}) {
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>메일</>}
        info={
          <>
            인증서 알림을 받을 주소를 등록합니다. 주소마다 받을 사건을 고릅니다.
          </>
        }
        actions={
          <>
            {" "}
            <div className={styles.headerActions}>
              <Link
                href="/admin/notifications/mail/new"
                className={buttonClassName({ variant: "primary" })}
              >
                <Icon name="plus" />새 수신처
              </Link>
            </div>{" "}
          </>
        }
      />

      {!mailer.ok ? (
        <Banner
          tone={"error"}
          title={
            <>SMTP 가 준비되지 않아 메일이 나가지 않습니다 — {mailer.message}</>
          }
          role="status"
        >
          <p className={styles.hint}>
            <Link href="/admin/settings/smtp" className={linkClassName()}>
              SMTP 설정
            </Link>
            을 마치면 등록해 둔 수신처가 그대로 동작합니다.
          </p>
        </Banner>
      ) : null}

      <Facts
        layout="strip"
        items={[
          { term: "수신처", value: recipients.length },
          {
            term: "사용",
            value: recipients.filter((item) => item.enabled).length,
          },
        ]}
      />

      <Surface title={<>수신처</>} as="section" flush>
        <Table label="메일 수신처">
          <thead>
            <Tr>
              <Th>수신 주소</Th>
              <Th>사건</Th>
              <Th>상태</Th>
              {/* 실제 알림 발송만 기록한다 — 테스트 발송은 여기 남지 않는다(상세 참조). */}
              <Th>마지막 알림</Th>
            </Tr>
          </thead>
          <tbody>
            {recipients.length === 0 ? (
              <Tr>
                <Td colSpan={4}>
                  <EmptyState
                    description={
                      <>
                        {" "}
                        등록된 수신처가 없습니다. 인증서 만기 안내를 받을 주소를
                        추가하세요.{" "}
                      </>
                    }
                  />
                </Td>
              </Tr>
            ) : null}
            {recipients.map((recipient) => {
              const events = (
                Array.isArray(recipient.events) ? recipient.events : []
              ) as string[];
              return (
                <Tr key={recipient.id}>
                  <Td cell="primary">
                    <Link href={`/admin/notifications/mail/${recipient.id}`}>
                      {recipient.label
                        ? `${recipient.label} <${recipient.address}>`
                        : recipient.address}
                    </Link>
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
                      tone={recipient.enabled ? "success" : "neutral"}
                      size="sm"
                    >
                      {recipient.enabled ? "사용" : "사용 안 함"}
                    </StatusBadge>
                  </Td>
                  <Td cell="meta" label="마지막 알림">
                    {!recipient.lastAttemptAt ? (
                      "아직 보낸 적 없음"
                    ) : recipient.lastError ? (
                      <StatusBadge tone={"error"} size="sm">
                        실패 ·{" "}
                        {formatServerTime(new Date(recipient.lastAttemptAt))}
                      </StatusBadge>
                    ) : (
                      <span>
                        성공 ·{" "}
                        {formatServerTime(new Date(recipient.lastAttemptAt))}
                      </span>
                    )}
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
