import type { Action, Role } from "@repo/contracts";

/** Presentation only. The API independently authorizes every read and mutation. */
const VISIBLE_ACTIONS: Record<Role, readonly Action[]> = {
  viewer: ["config:read"],
  editor: ["config:read", "config:write", "nginx:reload"],
  admin: ["config:read", "config:write", "nginx:reload", "user:manage"],
};

export function can(role: Role, action: Action): boolean {
  return VISIBLE_ACTIONS[role]?.includes(action) ?? false;
}
