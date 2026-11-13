export type OperationsTarget =
  { kind: "local" } | { kind: "node"; id: string; name: string };

export function targetLabel(target: OperationsTarget): string {
  return target.kind === "local" ? "로컬 서버" : target.name;
}
