import { StatusBadge } from "@repo/ui";
import type { Role } from "@repo/contracts";
import styles from "./role-badge.module.css";

export function RoleBadge({ role }: { role: Role }) {
  return (
    <StatusBadge tone="neutral" size="sm" dot={false} className={styles[role]}>
      {role}
    </StatusBadge>
  );
}
