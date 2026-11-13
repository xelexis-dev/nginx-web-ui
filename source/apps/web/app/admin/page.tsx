import { redirect } from "next/navigation";

// /admin has no landing of its own yet — go straight to the first (only)
// admin menu. The admin layout already enforces admin + 2FA.
export default function AdminIndexPage() {
  redirect("/admin/users");
}
