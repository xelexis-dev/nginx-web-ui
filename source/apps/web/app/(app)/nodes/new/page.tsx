import { readPageView } from "@/lib/api-page";
import { NodeNewView } from "../node-new-view";
export const runtime = "nodejs";
export default async function NewNodePage() {
  const { defaultGatewayName } = await readPageView<{ defaultGatewayName: string }>("/nodes/new");
  return <NodeNewView defaultGatewayName={defaultGatewayName} />;
}
