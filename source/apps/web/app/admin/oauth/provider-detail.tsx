import Link from "next/link";
import type { ReactNode } from "react";
import layout from "@/components/page-layout.module.css";
import {
  Facts,
  CodePane,
  Surface,
  StatusBadge,
  PageHeader,
  Icon,
  linkClassName,
} from "@repo/ui";
import type { ProviderView } from "./form-types";

import styles from "./oauth.module.css";

export function ProviderDetailView({
  provider,
  deleteForm,
}: {
  provider: ProviderView & { callbackUrl: string };
  deleteForm: ReactNode;
}) {
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>{provider.name}</>}
        description={<>{provider.id}</>}
        status={
          <StatusBadge tone={provider.enabled ? "success" : "neutral"}>
            {provider.enabled ? "사용" : "사용 안 함"}
          </StatusBadge>
        }
        eyebrow={
          <Link
            href="/admin/oauth"
            className={linkClassName({ variant: "back" })}
          >
            <Icon name="arrowLeft" size={14} />
            목록으로
          </Link>
        }
        actions={deleteForm}
      />

      <Surface title={<>설정</>} as="section">
        <Facts
          columns={2}
          items={[
            { term: <>로그인 서버 주소</>, value: <>{provider.issuer}</> },
            { term: <>클라이언트 ID</>, value: <>{provider.clientId}</> },
            {
              term: <>클라이언트 비밀키</>,
              value: (
                <span className={styles.secretValue}>
                  저장됨 (다시 볼 수 없습니다)
                </span>
              ),
            },
            { term: <>scope</>, value: <>{provider.scope}</> },
            {
              term: <>상태</>,
              value: (
                <>
                  <StatusBadge
                    tone={provider.enabled ? "success" : "neutral"}
                    size="sm"
                  >
                    {provider.enabled ? "사용" : "사용 안 함"}
                  </StatusBadge>
                </>
              ),
            },
            { term: <>연동된 사용자</>, value: <>{provider.linkedUsers}명</> },
          ]}
        />
      </Surface>

      <Surface as="section">
        <CodePane
          title="공급자 쪽에 등록할 콜백 주소"
          code={provider.callbackUrl}
          description={
            provider.callbackUrl.startsWith("http")
              ? "이 주소를 공급자의 「승인된 리디렉션 URI」에 그대로 넣으세요."
              : "이 앱의 공개 주소가 설정되지 않아 경로만 보여 줍니다. 「사이트 설정 › 일반」에서 공개 주소를 채우면 전체 주소로 나옵니다."
          }
        />
      </Surface>

      <Surface title={<>삭제</>} as="section">
        <p className={styles.pageNote}>
          {provider.linkedUsers > 0 ? (
            <>
              지금 <strong>{provider.linkedUsers}명</strong>이 이 공급자로
              연동돼 있습니다. 삭제하면 연동이 함께 지워지고,{" "}
              <strong>
                이 공급자로만 로그인하던 사람은 들어올 방법이 없어집니다
              </strong>
              (계정은 남습니다). 그 사람들에게 다른 로그인 방법이 있는지 먼저
              확인하세요.
            </>
          ) : (
            "이 공급자로 연동된 사용자가 없습니다. 삭제해도 잠기는 사람은 없습니다."
          )}
        </p>
      </Surface>
    </section>
  );
}

export { ProviderDetailActions } from "./provider-detail-actions";
