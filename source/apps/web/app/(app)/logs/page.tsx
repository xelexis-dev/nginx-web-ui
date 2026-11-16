import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
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
      <PageHeader title={<>로그</>} info={<>access.log / error.log를 실시간으로 확인합니다.</>} />

      <div>
        <LogViewer streamPath="/api/ui/v1/logs/stream" />
      </div>
    </section>
  );
}
