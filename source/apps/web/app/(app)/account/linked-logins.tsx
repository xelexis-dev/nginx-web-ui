import { formatServerTime } from "@/lib/format-time";
import { Button, Banner, Card } from "@repo/ui";
import { ApiForm } from "@/components/api-form";
import styles from "./linked-logins.module.css";

export interface LinkedLoginRow {
  providerId: string;
  name: string;
  linkedAt: Date | null;
}

/**
 * 계정 화면의 「연결된 로그인」. 관리자가 등록한 공급자마다 한 줄이고,
 * 연동 여부에 따라 「연동하기」/「연동 해제」가 붙는다.
 *
 * 해제 버튼은 마지막 남은 로그인 방법일 때 아예 렌더하지 않는다 — 눌러도
 * 서버가 막지만(link-actions), 누를 수 있게 두면 왜 안 되는지 알 수 없다.
 */
export function LinkedLogins({
  rows,
  hasPassword,
  notice,
}: {
  rows: LinkedLoginRow[];
  hasPassword: boolean;
  notice?: { kind: "linked" | "unlinked" | "last-method" };
}) {
  if (rows.length === 0) return null;

  const linkedCount = rows.filter((r) => r.linkedAt !== null).length;

  return (
    <Card
      id="account-logins"
      title="연결된 로그인"
      info="다른 서비스의 계정으로도 로그인할 수 있게 연결합니다."
    >
      <p className={styles.note}>
        연결해도 지금 쓰던
        {hasPassword
          ? " 비밀번호 로그인은 그대로 남습니다."
          : " 로그인 방법은 그대로 남습니다."}
      </p>

      {notice ? (
        <Banner
          tone={notice.kind === "last-method" ? "error" : "success"}
          title={
            <>
              {notice.kind === "linked"
                ? "연결했습니다. 이제 로그인 화면에서 그 버튼으로도 들어올 수 있습니다."
                : notice.kind === "unlinked"
                  ? "연결을 해제했습니다."
                  : "마지막 남은 로그인 방법은 해제할 수 없습니다. 먼저 다른 방법을 추가하세요."}
            </>
          }
        />
      ) : null}

      <ul className={styles.list}>
        {rows.map((row) => {
          const linked = row.linkedAt !== null;
          // 비밀번호가 없고 연결이 하나뿐이면, 그 하나가 유일한 문이다.
          const isLastMethod = linked && !hasPassword && linkedCount <= 1;

          return (
            <li key={row.providerId} className={styles.row}>
              <div className={styles.rowText}>
                <span className={styles.rowName}>{row.name}</span>
                <span className={styles.rowState}>
                  {linked
                    ? `연결됨 · ${formatServerTime(row.linkedAt!, { dateStyle: "medium" })}`
                    : "연결하지 않음"}
                </span>
              </div>

              {linked ? (
                isLastMethod ? (
                  <span className={styles.rowLocked}>유일한 로그인 방법</span>
                ) : (
                  <ApiForm action={{ id: "account.unlink", args: [] }}>
                    <input
                      type="hidden"
                      name="providerId"
                      value={row.providerId}
                    />
                    <Button type="submit" variant="secondary">
                      연결 해제
                    </Button>
                  </ApiForm>
                )
              ) : (
                <ApiForm action={{ id: "account.startLink", args: [] }}>
                  <input
                    type="hidden"
                    name="providerId"
                    value={row.providerId}
                  />
                  <Button type="submit" variant="secondary">
                    연결하기
                  </Button>
                </ApiForm>
              )}
            </li>
          );
        })}
      </ul>
    </Card>
  );
}
