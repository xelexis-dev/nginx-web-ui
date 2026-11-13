import { readPageApi } from "@/lib/api-page";
import type { PendingUserView } from "@/lib/admin-api";
import { ApiForm } from "@/components/api-form";
import { Button, Select } from "@repo/ui";
import styles from "../oauth/oauth.module.css";
import { PendingListView } from "./pending-list";
export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export default async function PendingUsersPage() {
  const pending = await readPageApi<PendingUserView[]>(
    "/api/ui/v1/admin/pending",
  );

  return (
    <PendingListView
      pending={pending}
      approvalForm={(u) => (
        <ApiForm
          action={{ id: "admin.pending.approve", args: [] }}
          id={`approve-${u.id}`}
          className={styles.actions}
        >
          <input type="hidden" name="id" value={u.id} />
          <Select
            size="sm"
            name="role"
            defaultValue="viewer"
            options={[
              { value: "viewer", label: "viewer" },
              { value: "editor", label: "editor" },
              { value: "admin", label: "admin" },
            ]}
            aria-label="승인할 등급"
          />
        </ApiForm>
      )}
      rejectionForm={(u) => (
        <ApiForm action={{ id: "admin.pending.reject", args: [] }}>
          <input type="hidden" name="id" value={u.id} />
          <Button size="sm" type="submit" variant="secondary">
            거절
          </Button>
        </ApiForm>
      )}
    />
  );
}
