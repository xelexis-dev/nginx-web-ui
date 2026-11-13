"use client";
import { useState } from "react";
import { Banner, Button, EmptyState, StatusBadge, Surface } from "@repo/ui";
import type { PasskeyListView } from "@/lib/passkey-api";
import { PasskeyTable } from "./passkey-list";
import { RemovePasskeyForm } from "./passkey-forms";
import styles from "./passkeys.module.css";

export function AdminPasskeys({
  data,
  userId,
  email,
  removed = false,
  isSelf = false,
}: {
  data: PasskeyListView;
  userId: string;
  email: string;
  removed?: boolean;
  isSelf?: boolean;
}) {
  const [removing, setRemoving] = useState<string | "all" | null>(null);
  const selected = data.passkeys.find((row) => row.id === removing);
  return (
    <Surface
      flush
      title={
        <span className={styles.title}>
          사용자의 패스키
          <StatusBadge tone="neutral" size="sm" dot={false}>
            {data.passkeys.length}개
          </StatusBadge>
        </span>
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
      {removed ? (
        <div className={styles.notice}>
          <Banner
            tone="success"
            title={isSelf
              ? "패스키를 삭제했습니다. 다른 기기의 로그인은 종료되고 이 기기의 로그인은 유지됩니다."
              : "패스키를 삭제하고 사용자의 로그인을 종료했습니다."}
          />
        </div>
      ) : null}
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
          available={data.availability.available}
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
        <EmptyState title="등록된 패스키가 없습니다" />
      )}
      {removing ? (
        <div className={styles.remove}>
          <h3>
            {removing === "all"
              ? `패스키 ${data.passkeys.length}개를 모두 삭제하시겠습니까?`
              : `${selected?.name ?? ""} 패스키를 삭제하시겠습니까?`}
          </h3>
          <RemovePasskeyForm
            key={removing}
            action={{ id: "admin.users.passkeys.remove", args: [] }}
            id={removing === "all" ? undefined : removing}
            all={removing === "all"}
            userId={userId}
            isSelf={isSelf}
            email={email}
            canRemove={data.canRemove}
            onCancel={() => setRemoving(null)}
          />
        </div>
      ) : null}
    </Surface>
  );
}
