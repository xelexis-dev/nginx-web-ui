import type { TokenApiRow } from "@repo/contracts";
export type { TokenApiRow, TokenApiList } from "@repo/contracts";
import { formatDateTime, type TokenRowView } from "@/app/(app)/account/tokens/tokens";
/** Only localised presentation runs in SSR. Status/masking come from the API. */
export function tokenPresentation(row: TokenApiRow): TokenRowView {
  const display = (value: string | null, fallback: string) => formatDateTime(value === null ? null : new Date(value), fallback);
  return { id: row.id, name: row.name, masked: row.masked, scopes: row.scopes, status: row.status,
    createdAt: display(row.createdAt, "-"), lastUsedAt: display(row.lastUsedAt, "사용한 적 없음"),
    expiresAt: display(row.expiresAt, "무기한"), revokedAt: display(row.revokedAt, "해지되지 않음"), revoked: row.revokedAt !== null };
}
