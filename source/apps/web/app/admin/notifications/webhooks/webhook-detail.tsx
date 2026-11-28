import Link from "next/link";
import type { ReactNode } from "react";
import { formatServerTime } from "@/lib/format-time";
import layout from "@/components/page-layout.module.css";
import {
  CodePane,
  Surface,
  Facts,
  StatusBadge,
  Banner,
  PageHeader,
  Icon,
  linkClassName,
} from "@repo/ui";
import { eventLabel } from "@repo/presentation/webhook";
import type { WebhookDetail } from "./form-types";
import styles from "../notifications.module.css";

const FORMAT_LABEL: Record<string, string> = {
  generic: "일반 (JSON)",
  discord: "Discord",
  dooray: "Dooray",
  custom: "일반 (JSON · 직접 작성)",
};

export function WebhookDetailView({
  target,
  maskedUrl,
  headerNames,
  events,
  detailActions,
}: {
  target: WebhookDetail;
  maskedUrl: WebhookDetail["maskedUrl"];
  headerNames: WebhookDetail["headerNames"];
  events: string[];
  detailActions: ReactNode;
}) {
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>{target.name}</>}
        status={
          <StatusBadge tone={target.enabled ? "success" : "neutral"} size="sm">
            {target.enabled ? "사용" : "사용 안 함"}
          </StatusBadge>
        }
        description={<>{FORMAT_LABEL[target.format] ?? target.format}</>}
        eyebrow={
          <Link
            href="/admin/notifications/webhooks"
            className={linkClassName({ variant: "back" })}
          >
            <Icon name="arrowLeft" size={14} />
            Webhook 수신처 목록
          </Link>
        }
        actions={detailActions}
      />

      {target.openError ? (
        <Banner
          tone={"error"}
          title={<>{target.openError}</>}
          role="alert"
        ></Banner>
      ) : null}

      <Surface title={<>설정</>} as="section">
        <Facts
          columns={2}
          items={[
            {
              term: <>URL</>,
              value: (
                <span className={styles.secretValue}>
                  {maskedUrl ?? "복호화할 수 없음"}
                </span>
              ),
            },
            {
              term: <>추가 헤더</>,
              value: (
                <>
                  {headerNames.length === 0 ? (
                    "없음"
                  ) : (
                    <span className={styles.secretValue}>
                      {headerNames.map((name) => `${name}: ******`).join(", ")}
                    </span>
                  )}
                </>
              ),
            },
            {
              term: <>보낼 사건</>,
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
                  {target.lastAttemptAt ? (
                    <>
                      {target.lastError ? (
                        <StatusBadge tone={"error"} size="sm">
                          실패
                        </StatusBadge>
                      ) : (
                        <span>성공 (HTTP {target.lastStatus ?? "?"})</span>
                      )}{" "}
                      · {formatServerTime(new Date(target.lastAttemptAt))}
                      {target.lastError ? (
                        <div className={styles.mono}>{target.lastError}</div>
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
                  {target.lastSuccessAt
                    ? formatServerTime(new Date(target.lastSuccessAt))
                    : "없음"}
                </>
              ),
            },
          ]}
        />
      </Surface>

      {target.format === "custom" && target.bodyTemplate ? (
        <Surface as="section">
          <CodePane title="본문" code={target.bodyTemplate} />
        </Surface>
      ) : null}

      <Surface title={<>동작</>} as="section">
        <p className={styles.hint}>
          테스트 전송은 <strong>실제로 한 건</strong>을 보내며, &quot;마지막
          알림&quot;에는 기록하지 않습니다.
        </p>
      </Surface>
    </section>
  );
}
