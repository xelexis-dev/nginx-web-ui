import { StatusBadge } from "@repo/ui";
import type { Role } from "@repo/contracts";

export function RoleBadge({ role }: { role: Role }) {
  return (
    <StatusBadge tone="neutral" size="sm" dot={false}>
      {role}
    </StatusBadge>
  );
}
