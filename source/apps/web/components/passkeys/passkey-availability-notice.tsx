"use client";
import { useSyncExternalStore } from "react";
import { Banner, StatusBadge, linkClassName } from "@repo/ui";
import { selectPasskeyNotice } from "@/lib/passkey-notice";
import styles from "./passkeys.module.css";
const subscribe = () => () => {};
const snapshot = () => window.location.href;
const serverSnapshot = () => null;
export function PasskeyAvailabilityNotice({
  reason,
  role,
}: {
  reason: string | null;
  role: string;
}) {
  const href = useSyncExternalStore(subscribe, snapshot, serverSnapshot);
  const notice = selectPasskeyNotice(reason, href ? new URL(href) : null, role);
  return (
    <div aria-busy={notice.pending || undefined}>
      <Banner tone="warning" title={notice.title}>
        <p>{notice.lead}</p>
        <div className={styles.noticeGrid}>
          <div>
            <h3 className={styles.noticeLabel}>확인된 내용</h3>
            <dl className={styles.diagnostics}>
              {notice.rows.map((row) => (
                <div key={row.label}>
                  <dt>{row.label}</dt>
                  <dd>
                    {row.tone ? (
                      <StatusBadge tone={row.tone} size="sm">
                        {row.value}
                      </StatusBadge>
                    ) : (
                      row.value
                    )}
                  </dd>
                </div>
              ))}
              {notice.pending ? (
                <div>
                  <dt>
                    {reason === "ip_host" ? "브라우저 주소" : "브라우저 연결"}
                  </dt>
                  <dd>
                    <span className={styles.skeleton}>
                      <span className={styles.visuallyHidden}>확인하는 중</span>
                    </span>
                  </dd>
                </div>
              ) : null}
            </dl>
            <p>{notice.cause}</p>
          </div>
          <div>
            <h3 className={styles.noticeLabel}>{notice.remedyLabel}</h3>
            {notice.pending ? (
              <div aria-label="확인하는 중" className={styles.skeletonLines}>
                <span className={styles.skeleton} />
                <span className={styles.skeleton} />
                {role === "admin" ? <span className={styles.skeleton} /> : null}
              </div>
            ) : notice.items.length > 1 ? (
              <ol className={styles.remedies}>
                {notice.items.map((item) => (
                  <li key={item}>{item}</li>
                ))}
              </ol>
            ) : (
              <p>
                {role === "admin" && notice.remedy === "U0" ? (
                  <>
                    <a
                      className={linkClassName()}
                      href="https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/README.ko.md#패스키로-로그인"
                      target="_blank"
                      rel="noreferrer"
                    >
                      설치 안내의 「패스키로 로그인」
                    </a>
                    에서 패스키를 쓸 수 있는 접속 조건을 확인하세요.
                  </>
                ) : (
                  notice.items[0]
                )}
              </p>
            )}
            {notice.footer ? (
              <p>
                자세한 설정은{" "}
                <a
                  className={linkClassName()}
                  href="https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/README.ko.md#패스키로-로그인"
                  target="_blank"
                  rel="noreferrer"
                >
                  설치 안내의 「패스키로 로그인」
                </a>
                에 있습니다.
              </p>
            ) : null}
          </div>
        </div>
      </Banner>
    </div>
  );
}
