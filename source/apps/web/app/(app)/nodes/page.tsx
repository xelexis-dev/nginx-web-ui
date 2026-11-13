import { readPageView } from "@/lib/api-page";
import { NodeList, type NodeListView } from "./node-list";
export const runtime = "nodejs";

export default async function NodesPage({ searchParams }: {
  searchParams: Promise<{ unpairWarning?: string }>;
}) {
  const data = await readPageView<NodeListView>("/nodes", await searchParams);
  return <NodeList {...data} />;
}
