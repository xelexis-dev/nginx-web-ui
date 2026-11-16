import type { ReactNode } from "react";
import { formatServerTime } from "@/lib/format-time";
import layout from "@/components/page-layout.module.css";
import type { PendingUserView } from "@/lib/admin-api";

import {
  EmptyState,
  Surface,
  Button,
  Table,
  Th,
  Td,
  Tr,
  PageHeader,
} from "@repo/ui";
import styles from "../oauth/oauth.module.css";

function formatDate(value: Date): string {
  return formatServerTime(value, { dateStyle: "medium", timeStyle: "short" });
}

export function PendingListView({
  pending,
  approvalForm,
  rejectionForm,
}: {
  pending: PendingUserView[];
  approvalForm: (u: PendingUserView) => ReactNode;
  rejectionForm: (u: PendingUserView) => ReactNode;
}) {
  return (
    <section className={layout.page}>
      <PageHeader
        title={
          <>
            승인 대기
            {pending.length > 0 ? (
              <span className={styles.rowNote}> · {pending.length}명</span>
            ) : null}
          </>
        }
        description={
          <>
            승인하기 전까지는 <strong>로그인 자체가 되지 않습니다</strong>.
            승인하면 다음 로그인부터 바로 이용합니다.
          </>
        }
        info={<>OAuth 로 처음 들어온 사람들입니다.</>}
      />

      <Surface as="section" flush>
        <Table label="승인 대기">
          <thead>
            <Tr>
              <Th>사용자</Th>
              <Th>공급자</Th>
              <Th>처음 시도</Th>
              <Th>등급</Th>
              <Th />
            </Tr>
          </thead>
          <tbody>
            {pending.length === 0 ? (
              <Tr>
                <Td colSpan={5}>
                  <EmptyState
                    description={<> 승인을 기다리는 사람이 없습니다. </>}
                  />
                </Td>
              </Tr>
            ) : null}
            {pending.map((u) => {
              const info = u;
              return (
                <Tr key={u.id}>
                  <Td cell="primary">
                    <div>{info?.displayName ?? u.email ?? "(이름 없음)"}</div>
                    {info?.email ? (
                      <div className={styles.rowNote}>{info.email}</div>
                    ) : null}
                  </Td>
                  <Td cell="meta" label="공급자">
                    {info?.provider ?? "—"}
                  </Td>
                  <Td cell="meta" label="처음 시도" className={styles.mono}>
                    {formatDate(new Date(u.createdAt))}
                  </Td>
                  <Td cell="meta" label="등급">
                    {/* 승인 폼과 같은 form 안에 있어야 고른 등급이 함께 전송된다. */}
                    {approvalForm(u)}
                  </Td>
                  <Td cell="action">
                    <div className={styles.actions}>
                      <Button
                        size="sm"
                        type="submit"
                        form={`approve-${u.id}`}
                        variant="secondary"
                      >
                        승인
                      </Button>
                      {rejectionForm(u)}
                    </div>
                  </Td>
                </Tr>
              );
            })}
          </tbody>
        </Table>
      </Surface>
      <p className={styles.hint}>
        거절해도 계정은 지우지 않고 잠급니다 — 같은 사람이 다시 로그인해도 새
        계정이 생기지 않아, 「언제 누가 들어오려 했는지」가 남습니다.
      </p>
    </section>
  );
}
