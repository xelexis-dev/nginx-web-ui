import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { readPageApi } from "@/lib/api-page";
import { readAdminSession, type AdminUserView } from "@/lib/admin-api";
import { EditUserForms } from "./edit-user-forms";

// Uses the authenticated Rust API view through the server-side page client.
export const runtime = "nodejs";

/**
 * Edit page for one user: 역할 변경 (role change) + 비밀번호 재설정
 * (password reset), combined per spec. Both forms redirect back to the
 * detail page (`/users/[id]`) on success; the last-admin-demotion guard
 * surfaces inline instead.
 */
export default async function EditUserPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  await readAdminSession();

  const user = await readPageApi<AdminUserView>(`/api/ui/v1/admin/users/${encodeURIComponent(id)}`);

  return (
    <section className={layout.page}>
      <PageHeader title={<>사용자 수정</>} />

      <div>
        <EditUserForms id={user.id} email={user.email} currentRole={user.role} hasPassword={user.hasPassword} />
      </div>
    </section>
  );
}
