import { FormStack, Banner } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { readAdminSession } from "@/lib/admin-api";
import { UserForm } from "../user-form";

// Uses the authenticated Rust API view through the server-side page client.
export const runtime = "nodejs";

/** Create page for a new user. On success `createUser` redirects to the new user's detail page. */
export default async function NewUserPage() {
  await readAdminSession();

  return (
    <section className={layout.page}>
      <PageHeader title={<>새 사용자</>} />

      <FormStack>
        <Banner
          tone="info"
          title={<>이메일, 비밀번호, 역할을 지정해 새 계정을 생성합니다.</>}
        />
        <UserForm />
      </FormStack>
    </section>
  );
}
