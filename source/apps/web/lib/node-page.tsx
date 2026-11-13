import type { ReactNode } from "react";
import type { Action, Role, NodeView, RuntimeDnsStatus, RemoteNodeViewSuccess } from "@repo/contracts";
export type { NodeView } from "@repo/contracts";
import { NodeErrorCard, NODE_UNEXPECTED_RESPONSE_MESSAGE, type NodeErrorCardProps } from "@/components/node-error";
import { readPageView } from "./api-page";

export type NodePageAuth =
  | { ok: true; node: NodeView; effectiveRole: Role; runtimeDns?: RuntimeDnsStatus }
  | { ok: false; render: ReactNode };
export type NodePageData<T> =
  | { ok: true; node: NodeView; effectiveRole: Role; data: T; runtimeDns?: RuntimeDnsStatus }
  | { ok: false; render: ReactNode };
type ApiNodeView =
  | RemoteNodeViewSuccess
  | { ok: false; nodeError: NodeErrorCardProps };
interface PageContext {
  nodeId: string;
  action: Action;
  retryHref: string;
  listHref: string;
}
export interface LoadNodePageArgs<T> extends PageContext {
  /** Legacy presentation metadata; API's static page registry owns outbound requests. */
  path: string;
  toViewModel: (raw: unknown) => T;
  timeoutMs?: number;
}
export interface LoadNodePageAllArgs<K extends string, T> extends PageContext {
  paths: Record<K, string>;
  subjects: Record<K, string>;
  toViewModel: (raw: Record<K, unknown>) => T;
  timeoutMs?: number;
}

async function read(context: PageContext): Promise<ApiNodeView> {
  const url = new URL(context.retryHref, "https://ui.invalid");
  const prefix = `/nodes/${context.nodeId}`;
  if (url.origin !== "https://ui.invalid" || !(url.pathname === prefix || url.pathname.startsWith(prefix + "/"))) {
    throw new Error("Invalid node page route");
  }
  return readPageView<ApiNodeView>(url.pathname, Object.fromEntries(url.searchParams));
}

/** Authorization, bounded parallel reads and remote error priority belong to Rust. */
export async function authorizeNodePage(context: PageContext): Promise<NodePageAuth> {
  const result = await read(context);
  return result.ok
    ? { ok: true, node: result.node, effectiveRole: result.effectiveRole, runtimeDns: result.runtimeDns }
    : { ok: false, render: <NodeErrorCard {...result.nodeError} /> };
}

async function load<T>(context: PageContext, convert: (raw: unknown) => T): Promise<NodePageData<T>> {
  const result = await read(context);
  if (!result.ok) return { ok: false, render: <NodeErrorCard {...result.nodeError} /> };
  try {
    return { ok: true, node: result.node, effectiveRole: result.effectiveRole, data: convert(result.data), runtimeDns: result.runtimeDns };
  } catch {
    return { ok: false, render: <NodeErrorCard
      failure={{kind:"node_error",message:NODE_UNEXPECTED_RESPONSE_MESSAGE}}
      nodeId={context.nodeId} retryHref={context.retryHref} listHref={context.listHref} /> };
  }
}
export function loadNodePage<T>(args: LoadNodePageArgs<T>): Promise<NodePageData<T>> {
  return load(args, args.toViewModel);
}
export function loadNodePageAll<K extends string,T>(args: LoadNodePageAllArgs<K,T>): Promise<NodePageData<T>> {
  return load(args, raw => args.toViewModel(raw as Record<K,unknown>));
}
