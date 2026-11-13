import { formatServerTime } from "@/lib/format-time";
import { readPageApi } from "@/lib/api-page";
import { readAdminSession, type AdminUserListItem } from "@/lib/admin-api";
import { UserList, type UserRowVM } from "./user-list";
export const runtime = "nodejs";

function formatDate(value: Date): string {
  return formatServerTime(value, { dateStyle: "medium", timeStyle: "short" });
}

/**
 * List-only page (F7): a data-table of users. Create/edit are separate
 * pages now (`/admin/users/new`, `/admin/users/[id]/edit`); each row links to its
 * detail page (`/admin/users/[id]`), which hosts the role-change/reset/delete
 * actions.
 */
export default async function UsersPage() {
  const session = await readAdminSession();
  const rows = await readPageApi<AdminUserListItem[]>("/api/ui/v1/admin/users");
  const items: UserRowVM[] = rows.map((u) => ({
    ...u,
    methods: [u.hasPassword ? "비밀번호" : null, ...u.providers].filter(Boolean).join(" · ") || "없음",
    createdAt: formatDate(new Date(u.createdAt)),
  }));

  return <UserList items={items} selfUserId={session.user.id} />;
}
