import type { ReactNode } from "react";
import { formatServerTime } from "@/lib/format-time";
import layout from "@/components/page-layout.module.css";
import {
  Icon,
  Surface,
  Facts,
  StatusBadge,
  linkClassName,
  PageHeader,
} from "@repo/ui";
import Link from "next/link";
import type { AdminUserView } from "@/lib/admin-api";
import { RoleBadge } from "@/components/role-badge";
import styles from "./users.module.css";

// Uses the authenticated Rust API view through the server-side page client.

function formatDate(value: Date): string {
  return formatServerTime(value, { dateStyle: "medium", timeStyle: "short" });
}

/** Detail page for one user: email/role/created + 수정/삭제 actions. NEVER renders the password hash. */
export function UserDetailView({
  user,
  isSelf,
  detailActions,
  passkeys,
  twoFactor,
  result,
}: {
  user: AdminUserView;
  isSelf: boolean;
  detailActions: ReactNode;
  passkeys?: ReactNode;
  twoFactor?: ReactNode;
  result?: ReactNode;
}) {
  return (
    <section className={layout.page}>
      <PageHeader
        title={
          <>
            {user.email}
            {isSelf ? <span className={styles.youTag}>(나)</span> : null}
          </>
        }
        status={
          <StatusBadge tone={user.enabled ? "success" : "neutral"}>
            {user.enabled ? "활성" : "비활성"}
          </StatusBadge>
        }
        eyebrow={
          <>
            {" "}
            <Link
              href="/admin/users"
              className={linkClassName({ variant: "back" })}
            >
              <Icon name="arrowLeft" size={14} />
              목록으로
            </Link>{" "}
          </>
        }
        actions={detailActions}
      />

      {result}
      <Surface as="div">
        <Facts
          columns={2}
          items={[
            { term: <>이메일</>, value: <>{user.email}</> },
            {
              term: <>역할</>,
              value: (
                <>
                  <RoleBadge role={user.role} />
                </>
              ),
            },
            {
              term: <>생성일</>,
              value: <>{formatDate(new Date(user.createdAt))}</>,
            },
            {
              term: <>상태</>,
              value: (
                <>
                  <StatusBadge
                    tone={user.enabled ? "success" : "neutral"}
                    size="sm"
                  >
                    {user.enabled ? "활성" : "비활성"}
                  </StatusBadge>
                </>
              ),
            },
          ]}
        />
      </Surface>

      {twoFactor}
      {passkeys}
    </section>
  );
}
