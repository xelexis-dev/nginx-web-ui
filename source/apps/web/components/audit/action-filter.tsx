"use client";

import { useRouter } from "next/navigation";
import { Select } from "@repo/ui";
import styles from "./audit.module.css";

/**
 * Action dropdown for the audit log. Client component because changing the
 * filter needs to navigate (reset to page 1, replace `?action=`) as soon as
 * the user picks a value — no separate "apply" button needed.
 *
 * `basePath` is `/audit` for the local screen and `/nodes/<id>/audit` for the
 * node-context screen — same reason `AuditList` takes it (audit-list.tsx).
 */
export function ActionFilter({
  actions,
  selected,
  basePath,
}: {
  actions: string[];
  selected: string;
  basePath: string;
}) {
  const router = useRouter();

  function onChange(value: string) {
    const params = new URLSearchParams();
    if (value) params.set("action", value);
    const qs = params.toString();
    router.push(qs ? `${basePath}?${qs}` : basePath);
  }

  return (
    <div className={styles.filter}>
      <Select
        aria-label="액션 필터"
        value={selected}
        onChange={onChange}
        placeholder="전체 액션"
        options={[{ value: "", label: "전체 액션" }, ...actions.map((a) => ({ value: a, label: a }))]}
      />
    </div>
  );
}
