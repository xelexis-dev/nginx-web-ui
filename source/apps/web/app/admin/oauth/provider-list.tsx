import type { ReactNode } from "react";
import layout from "@/components/page-layout.module.css";
import {
  Icon,
  Facts,
  StatusBadge,
  EmptyState,
  Surface,
  buttonClassName,
  Table,
  Th,
  Td,
  Tr,
  PageHeader,
} from "@repo/ui";
import Link from "next/link";

import type { ProviderView } from "./form-types";
import styles from "./oauth.module.css";

// Admin-only. requireRole gates the route server-side (defense in depth alongside
// the admin layout's guard and each action's own guard).
// Reads live DB state on every request; must not be prerendered at build time.

export function ProviderListView({
  providers,
  toggleForm,
}: {
  providers: ProviderView[];
  toggleForm: (p: ProviderView) => ReactNode;
}) {
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>OAuth 로그인</>}
        info={
          <>
            다른 서비스의 계정으로 로그인할 수 있게 공급자를 등록합니다. OIDC 를
            지원하는 곳이면 무엇이든 됩니다 — Google · Keycloak · Okta · 자체
            로그인 서버 등. 등록하지 않으면 로그인 화면은 지금과 똑같습니다.
          </>
        }
        actions={
          <>
            {" "}
            <div className={styles.headerActions}>
              <Link
                href="/admin/oauth/new"
                className={buttonClassName({ variant: "primary" })}
              >
                <Icon name="plus" />
                공급자 추가
              </Link>
            </div>{" "}
          </>
        }
      />

      <Facts
        layout="strip"
        items={[
          { term: "공급자", value: providers.length },
          {
            term: "사용",
            value: providers.filter((item) => item.enabled).length,
          },
        ]}
      />

      <Surface title={<>공급자</>} as="section" flush>
        <Table label="OAuth 공급자" density="comfortable" narrow="stacked">
          <thead>
            <Tr>
              <Th>이름</Th>
              <Th>로그인 서버 주소</Th>
              <Th>연동된 사용자</Th>
              <Th>상태</Th>
              <Th />
            </Tr>
          </thead>
          <tbody>
            {providers.length === 0 ? (
              <Tr>
                <Td colSpan={5}>
                  <EmptyState
                    description={
                      <>
                        {" "}
                        등록된 공급자가 없습니다. 추가하면 로그인 화면에 버튼이
                        생깁니다.{" "}
                      </>
                    }
                  />
                </Td>
              </Tr>
            ) : null}
            {providers.map((p) => (
              <Tr key={p.id}>
                <Td cell="primary">
                  <Link href={`/admin/oauth/${p.id}`}>{p.name}</Link>
                  <div className={styles.rowNote}>{p.id}</div>
                </Td>
                <Td
                  cell="meta"
                  label="로그인 서버 주소"
                  className={styles.mono}
                >
                  {p.issuer}
                </Td>
                <Td cell="meta" label="연동된 사용자">
                  {p.linkedUsers}명
                </Td>
                <Td cell="status" label="상태">
                  <StatusBadge
                    tone={p.enabled ? "success" : "neutral"}
                    size="sm"
                  >
                    {p.enabled ? "사용" : "사용 안 함"}
                  </StatusBadge>
                </Td>
                <Td cell="action" label="작업">
                  {/* 설정을 지우지 않고 버튼만 감추는 스위치. */}
                  {toggleForm(p)}
                </Td>
              </Tr>
            ))}
          </tbody>
        </Table>
      </Surface>
    </section>
  );
}
