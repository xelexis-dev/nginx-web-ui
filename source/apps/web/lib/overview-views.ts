/** Read-only browser projections generated from the API schema. */
export type {
  DashboardView,
  AuditRowView,
  AuditListView,
  AuditDetailView,
  HistoryView,
  HistoryDetailView,
} from "@repo/contracts";

export interface RollbackViewResult {
  ok: boolean; output?: string; error?: string; kind?: "not_found" | "db_restore_failed";
}
