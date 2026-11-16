"use client";
import { useActionState } from "react";
import type { BasicFormState } from "@repo/contracts";
import { formatServerTime } from "@/lib/format-time";
import { useApiAction } from "@/lib/use-api-action";
import {
  Button,
  Banner,
  SettingRow,
  SettingRows,
  StatusBadge,
  Surface,
} from "@repo/ui";
import styles from "./linked-logins.module.css";

export interface LinkedLogin {
  providerId: string;
  name: string;
  linkedAt: Date | null;
}
export function LinkedLoginRow({
  row,
  isLastMethod,
}: {
  row: LinkedLogin;
  isLastMethod: boolean;
}) {
  const linked = row.linkedAt !== null;
  const [state, action, pending] = useActionState(
    useApiAction<BasicFormState>({
      id: linked ? "account.unlink" : "account.startLink",
      args: [],
    }),
    { ok: false },
  );
  return (
    <SettingRow
      label={row.name}
      status={
        <StatusBadge tone={linked ? "success" : "neutral"}>
          {linked ? "연결됨" : "연결 안 됨"}
        </StatusBadge>
      }
      detail={
        linked
          ? `${formatServerTime(row.linkedAt!, { dateStyle: "medium" })} 연결${isLastMethod ? " · 마지막 남은 로그인 방법이라 해제할 수 없습니다." : ""}`
          : undefined
      }
      action={
        !isLastMethod ? (
          <form action={action}>
            <input type="hidden" name="providerId" value={row.providerId} />
            <Button
              type="submit"
              variant="secondary"
              size="sm"
              disabled={pending}
            >
              {linked ? "연결 해제" : "연결하기"}
            </Button>
          </form>
        ) : undefined
      }
      message={
        state.error ? <Banner tone="error" title={state.error} /> : undefined
      }
    />
  );
}
export function LinkedLogins({
  rows,
  hasPassword,
}: {
  rows: LinkedLogin[];
  hasPassword: boolean;
}) {
  const count = rows.filter((row) => row.linkedAt !== null).length;
  return (
    <Surface
      id="account-logins"
      title="연결된 로그인"
      description="연결한 외부 계정으로도 이 계정에 로그인할 수 있습니다."
    >
      {rows.length ? (
        <SettingRows>
          {rows.map((row) => (
            <LinkedLoginRow
              key={row.providerId}
              row={row}
              isLastMethod={row.linkedAt !== null && !hasPassword && count <= 1}
            />
          ))}
        </SettingRows>
      ) : (
        <p className={styles.note}>관리자가 설정한 외부 로그인이 없습니다.</p>
      )}
    </Surface>
  );
}
