import Link from "next/link";
import type { ReactNode } from "react";
import { formatServerTime } from "@/lib/format-time";
import layout from "@/components/page-layout.module.css";
import {
  Surface,
  Facts,
  StatusBadge,
  PageHeader,
  Icon,
  linkClassName,
} from "@repo/ui";
import { eventLabel } from "@repo/presentation/webhook";
import type { MailRecipientRow } from "./form-types";
import styles from "../notifications.module.css";

export function RecipientDetailView({
  recipient,
  events,
  detailActions,
}: {
  recipient: MailRecipientRow;
  events: string[];
  detailActions: ReactNode;
}) {
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>{recipient.label || recipient.address}</>}
        status={
          <StatusBadge
            tone={recipient.enabled ? "success" : "neutral"}
            size="sm"
          >
            {recipient.enabled ? "사용" : "사용 안 함"}
          </StatusBadge>
        }
        description={<>{recipient.label ? recipient.address : "메일 수신처"}</>}
        eyebrow={
          <Link
            href="/admin/notifications/mail"
            className={linkClassName({ variant: "back" })}
          >
            <Icon name="arrowLeft" size={14} />
            메일 수신처 목록
          </Link>
        }
        actions={detailActions}
      />

      <Surface title={<>설정</>} as="section">
        <Facts
          columns={2}
          items={[
            { term: <>수신 주소</>, value: <>{recipient.address}</> },
            { term: <>표시 이름</>, value: <>{recipient.label || "—"}</> },
            {
              term: <>받을 사건</>,
              value: (
                <>
                  <span className={styles.chips}>
                    {events.length === 0
                      ? "없음"
                      : events.map((event) => (
                          <StatusBadge
                            tone={"neutral"}
                            size="sm"

                            key={event}
                          >
                            {eventLabel(event)}
                          </StatusBadge>
                        ))}
                  </span>
                </>
              ),
            },
            {
              term: <>마지막 알림</>,
              value: (
                <>
                  {recipient.lastAttemptAt ? (
                    <>
                      {recipient.lastError ? (
                        <StatusBadge tone={"error"} size="sm">
                          실패
                        </StatusBadge>
                      ) : (
                        <span>성공</span>
                      )}{" "}
                      · {formatServerTime(new Date(recipient.lastAttemptAt))}
                      {recipient.lastError ? (
                        <div className={styles.mono}>{recipient.lastError}</div>
                      ) : null}
                    </>
                  ) : (
                    "아직 보낸 적 없음"
                  )}
                </>
              ),
            },
            {
              term: <>마지막 성공</>,
              value: (
                <>
                  {recipient.lastSuccessAt
                    ? formatServerTime(new Date(recipient.lastSuccessAt))
                    : "없음"}
                </>
              ),
            },
          ]}
        />
      </Surface>

      <Surface title={<>동작</>} as="section">
        <p className={styles.hint}>
          테스트 발송은 <strong>실제로 한 통</strong>을 보내며, &quot;마지막
          알림&quot;에는 기록하지 않습니다.
        </p>
      </Surface>
    </section>
  );
}
