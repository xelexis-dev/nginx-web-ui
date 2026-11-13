import { FormStack, Banner } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { buttonClassName, PageHeader } from "@repo/ui";
import Link from "next/link";
import { readAdminSession } from "@/lib/admin-api";
import { ProviderForm } from "../provider-form";
import styles from "../oauth.module.css";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export default async function NewOAuthProviderPage() {
  await readAdminSession();

  return (
    <section className={layout.page}>
      <PageHeader
        title={<>공급자 추가</>}

        actions={
          <>
            {" "}
            <div className={styles.headerActions}>
              <Link
                href="/admin/oauth"
                className={buttonClassName({ variant: "secondary" })}
              >
                목록으로
              </Link>
            </div>{" "}
          </>
        }
      />

      <FormStack>
        <Banner
          tone="info"
          title={
            <>
              공급자 쪽에서 애플리케이션을 먼저 만들고, 거기서 받은 클라이언트
              ID 와 비밀키를 넣으세요. 저장하면 등록해야 할 콜백 주소를 알려
              드립니다.
            </>
          }
        />
        <ProviderForm mode="create" />
      </FormStack>
    </section>
  );
}
