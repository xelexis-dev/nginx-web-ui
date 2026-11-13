import { readPageView } from "@/lib/api-page";
import { NodeOverview, type NodeOverviewView } from "./node-overview";
export const runtime = "nodejs";

// A single read: nginx -t must never be triggered by a polling loop.
export default async function NodeDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const result = await readPageView<NodeOverviewView>(`/nodes/${id}`);
  return <NodeOverview id={id} result={result} />;
}
