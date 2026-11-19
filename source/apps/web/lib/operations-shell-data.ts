import type { ActionRef, ConfigDiff, Role, ServerListView } from "@repo/contracts";
import { readView } from "./api-client";
import { readConfigDiff } from "./config-diff";
import { can } from "./permissions";

export type OperationsShellData = {
  available: true;
  nodeVersion?: string | null;
  serverTotal: number;
  serverActive: number;
  pendingCount: number;
  pendingHasBaseline: boolean;
  pendingDiff: ConfigDiff;
  canApply: boolean;
  applyAction: ActionRef;
} | { available: false };

type RemoteList = { ok: false } | {
  ok: true;
  effectiveRole: Role;
  node?: { lastVersion?: string | null };
  data: {
    servers: { servers: { enabled: boolean }[] };
    pending: { count: number; hasBaseline?: boolean; diff: ConfigDiff };
  };
};

/** Optional shell information. Never query /status (which runs nginx -t), and
 * never fall back to local data when a node is unavailable. router.refresh after
 * a mutation re-renders the layout and delivers a new snapshot to the shell. */
export async function readOperationsShellData(nodeId?: string): Promise<OperationsShellData> {
  try {
    const basePath = nodeId ? `/nodes/${nodeId}/reverse-proxy` : "/reverse-proxy";
    if (nodeId) {
      const view = await readView<RemoteList>(basePath);
      if (!view.ok) return { available: false };
      const data = snapshot(view.data.servers.servers, view.data.pending,
        can(view.effectiveRole, "nginx:reload"),
        { id: "node.applyNodeConfig", args: [nodeId, basePath] });
      return data.available ? { ...data, nodeVersion: view.node?.lastVersion ?? null } : data;
    }
    const view = await readView<ServerListView>(basePath);
    return snapshot(view.servers, view.pending, view.canApply,
      { id: "apply.applyConfig", args: [] });
  } catch {
    // The authenticated layout's existing session guard owns redirects. Failure
    // of this secondary read must not hide the page or imply zero/healthy state.
    return { available: false };
  }
}

function snapshot(servers: { enabled: boolean }[], pending: { count: number; hasBaseline?: boolean; diff: ConfigDiff },
  canApply: boolean, applyAction: ActionRef): OperationsShellData {
  if (!Array.isArray(servers) || servers.some(server => typeof server.enabled !== "boolean") ||
      !Number.isSafeInteger(pending.count) || pending.count < 0) return { available: false };
  return {
    available: true,
    serverTotal: servers.length,
    serverActive: servers.filter(server => server.enabled).length,
    pendingCount: pending.count,
    pendingHasBaseline: pending.hasBaseline === true,
    pendingDiff: readConfigDiff(pending.diff),
    canApply,
    applyAction,
  };
}
