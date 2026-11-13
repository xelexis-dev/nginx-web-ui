import type { TokenApiRow } from "@repo/contracts";
/** Date-valued token row used by UI test fixtures. */
export type TokenListRow = Omit<TokenApiRow, "status" | "createdAt" | "lastUsedAt" | "expiresAt" | "revokedAt"> & {
  createdAt: Date; lastUsedAt: Date | null; expiresAt: Date | null; revokedAt: Date | null;
};
