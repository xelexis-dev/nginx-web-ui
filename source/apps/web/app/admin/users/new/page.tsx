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

      <UserForm />
    </section>
  );
}
