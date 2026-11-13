import type { TokenListRow } from "@/lib/token-presentation";
import type { TokenApiRow } from "@/lib/token-api";
import type { Role, TokenScope } from "@repo/contracts";

/** Fixed API fixture metadata; the backend tests own authorization and issuance policy. */
export function tokenScopeFixture(kind: "user" | "server", role: Role): readonly TokenScope[] {
  const user: Record<Role, readonly TokenScope[]> = {
    viewer: ["config:read"],
    editor: ["config:read", "config:write", "nginx:reload"],
    admin: ["config:read", "config:write", "nginx:reload", "user:manage"],
  };
  return kind === "server"
    ? ["config:read", "config:write", "nginx:reload", "user:manage", "node:pair"]
    : user[role];
}

/** Wire-format fixture conversion; authorization is tested against the Rust API. */
export function tokenWireFixture(row: TokenListRow): TokenApiRow {
  return {
    ...row,
    status: row.revokedAt ? "revoked" : row.expiresAt && row.expiresAt <= new Date() ? "expired" : "active",
    createdAt: row.createdAt.toISOString(),
    lastUsedAt: row.lastUsedAt?.toISOString() ?? null,
    expiresAt: row.expiresAt?.toISOString() ?? null,
    revokedAt: row.revokedAt?.toISOString() ?? null,
  };
}
