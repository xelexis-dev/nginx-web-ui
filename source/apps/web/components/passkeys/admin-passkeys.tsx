"use client";
import { useState } from "react";
import { Banner, Button, EmptyState, StatusBadge, Surface } from "@repo/ui";
import type { PasskeyListView } from "@/lib/passkey-api";
import { PasskeyTable } from "./passkey-list";
import { RemovePasskeyDialog } from "./remove-passkey-dialog";
import styles from "./passkeys.module.css";

export function AdminPasskeys({
  data,
  userId,
  email,
  isSelf = false,
}: {
  data: PasskeyListView;
  userId: string;
  email: string;
  isSelf?: boolean;
}) {
  const [removing, setRemoving] = useState<string | "all" | null>(null);
  const selected = data.passkeys.find((row) => row.id === removing);
  return (
    <Surface
      flush
      title="사용자의 패스키"
      tag={
        <StatusBadge tone="neutral" size="sm" dot={false}>
          {data.passkeys.length}개
        </StatusBadge>
      }
      actions={
        <Button
          variant="secondary"
          size="sm"
          disabled={!data.canRemove || !data.passkeys.length}
          onClick={() => setRemoving("all")}
        >
          전체 삭제
        </Button>
      }
    >
      {!data.canRemove ? (
        <div className={styles.notice}>
          <Banner
            tone="info"
            title="외부 로그인 전용 관리자 계정은 패스키를 삭제할 수 없습니다."
          >
            비밀번호를 사용하는 관리자에게 요청하세요.
          </Banner>
        </div>
      ) : null}
      {data.passkeys.length ? (
        <PasskeyTable
          rows={data.passkeys}
          action={(row) => (
            <Button
              variant="secondary"
              size="sm"
              disabled={!data.canRemove}
              onClick={() => setRemoving(row.id)}
            >
              삭제
            </Button>
          )}
        />
      ) : (
        <EmptyState description="등록된 패스키가 없습니다." />
      )}
      {removing ? (
        <RemovePasskeyDialog
          key={removing}
          action={{ id: "admin.users.passkeys.remove", args: [] }}
          id={removing === "all" ? undefined : removing}
          name={selected?.name}
          count={data.passkeys.length}
          all={removing === "all"}
          userId={userId}
          isSelf={isSelf}
          email={email}
          onClose={() => setRemoving(null)}
        />
      ) : null}
    </Surface>
  );
}
