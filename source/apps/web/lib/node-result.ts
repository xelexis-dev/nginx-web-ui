/** Safe remote API result types; transport and credentials live in Rust. */
export type { NodeStatus } from "@repo/contracts";

export type NodeFailure =
  | { kind: "unreachable"; message: string }
  | { kind: "unauthorized"; message: string }
  | { kind: "forbidden"; message: string }
  | { kind: "version_mismatch"; message: string; nodeVersion: string | null }
  | { kind: "too_old"; message: string }
  | { kind: "conflict"; message: string }
  | { kind: "not_found"; message: string }
  | { kind: "validation"; message: string; fieldErrors: Record<string, string> }
  /** nginx validation/reload failure; its process output is the message. */
  | { kind: "domain_failure"; message: string }
  | { kind: "rate_limited"; message: string }
  | { kind: "node_error"; message: string };

export type NodeResult<T> =
  | { ok: true; value: T; nodeVersion: string | null }
  | ({ ok: false; nodeVersion: string | null } & NodeFailure);
