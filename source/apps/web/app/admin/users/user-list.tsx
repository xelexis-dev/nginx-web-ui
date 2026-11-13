import layout from "@/components/page-layout.module.css";
import {
  Icon,
  Facts,
  StatusBadge,
  Surface,
  buttonClassName,
  Table,
  Th,
  Td,
  Tr,
  PageHeader,
} from "@repo/ui";
import Link from "next/link";
import { RoleBadge } from "@/components/role-badge";
import styles from "./users.module.css";

export interface UserRowVM {
  id: string;
  /** OAuth 로만 만들어진 계정은 이메일이 없다 — 그 자리에 공급자가 준 이름을 쓴다. */
  email: string | null;
  displayName: string | null;
  role: "admin" | "editor" | "viewer";
  enabled: boolean;
  /** 관리자 승인 여부. 등급과 독립이며, false 면 로그인 자체가 되지 않는다. */
  approved: boolean;
  /** 쓸 수 있는 로그인 방법을 사람이 읽는 문자열로: "비밀번호 · Google" */
  methods: string;
  createdAt: string;
}

export function UserList({
  items,
  selfUserId,
}: {
  items: UserRowVM[];
  selfUserId: string;
}) {
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>사용자</>}
        description={
          <>본인 계정 삭제와 마지막 admin 삭제/강등은 서버에서 차단됩니다.</>
        }
        info={<>관리자 계정을 생성, 역할 변경, 비밀번호 재설정, 삭제합니다.</>}
        actions={
          <>
            {" "}
            <div className={styles.listActions}>
              <Link
                href="/admin/users/new"
                className={buttonClassName({ variant: "primary" })}
              >
                <Icon name="plus" />새 사용자
              </Link>
            </div>{" "}
          </>
        }
      />

      <Facts
        layout="strip"
        items={[
          { term: "사용자", value: items.length },
          {
            term: "활성",
            value: items.filter((item) => item.enabled && item.approved).length,
          },
        ]}
      />

      <Surface flush>
        <Table label="사용자">
          <thead>
            <Tr>
              <Th>사용자</Th>
              <Th>로그인 방식</Th>
              <Th>역할</Th>
              <Th>상태</Th>
              <Th>생성일</Th>
            </Tr>
          </thead>
          <tbody>
            {items.length === 0 ? (
              <Tr>
                <Td colSpan={5} className={styles.emptyCell}>
                  등록된 사용자가 없습니다.
                </Td>
              </Tr>
            ) : null}
            {items.map((user) => {
              const isSelf = user.id === selfUserId;
              return (
                <Tr key={user.id}>
                  <Td
                    cell="primary"
                    icon={
                      <span className={styles.avatar}>
                        {(user.displayName ?? user.email ?? "?").slice(0, 1)}
                      </span>
                    }
                    secondary={
                      user.displayName && user.email ? user.email : undefined
                    }
                  >
                    <Link href={`/admin/users/${user.id}`}>
                      {user.displayName ?? user.email ?? "(이름 없음)"}
                    </Link>
                    {isSelf ? (
                      <span className={styles.youTag}>(나)</span>
                    ) : null}
                  </Td>
                  <Td
                    cell="meta"
                    label="로그인 방식"
                    className={styles.methods}
                  >
                    {user.methods}
                  </Td>
                  <Td cell="meta" label="역할">
                    <RoleBadge role={user.role} />
                  </Td>
                  <Td cell="status">
                    {!user.approved ? (
                      <StatusBadge tone={"warning"} size="sm">
                        승인 대기
                      </StatusBadge>
                    ) : (
                      <StatusBadge
                        tone={user.enabled ? "success" : "neutral"}
                        size="sm"
                      >
                        {user.enabled ? "활성" : "비활성"}
                      </StatusBadge>
                    )}
                  </Td>
                  <Td cell="meta" label="생성일">
                    {user.createdAt}
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
