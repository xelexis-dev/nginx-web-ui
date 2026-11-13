import { readPageView } from "@/lib/api-page";
import type { NodeView } from "@/lib/node-page";
import { NodeEditView } from "./node-edit-view";
export const runtime = "nodejs";
export default async function EditNodePage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const { node } = await readPageView<{ node: NodeView }>(`/nodes/${id}/edit`);
  return <NodeEditView node={node} />;
}
