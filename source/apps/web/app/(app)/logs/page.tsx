import layout from "@/components/page-layout.module.css";
import { readPageView } from "@/lib/api-page";
import { LogViewer } from "@/components/logs/log-viewer";

// Full F5 log viewer — tails access.log/error.log live via the
// /api/ui/v1/logs/stream SSE route (RBAC-checked there too, since EventSource
// can't carry custom headers for a second server-side check on each event).
export const runtime = "nodejs";

export default async function LogsPage() {
  await readPageView("/logs");

  return (
    <section className={layout.page}>
      <LogViewer streamPath="/api/ui/v1/logs/stream" />
    </section>
  );
}
