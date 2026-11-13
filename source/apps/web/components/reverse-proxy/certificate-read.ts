import type { CertificateListView } from "@repo/contracts";
import { readInstant } from "@/components/certificates/certificate-vm";
import { readOptionalView } from "@/lib/optional-read";
import type { CertificateSummary } from "./server-status";

/**
 * 서버 목록의 TLS 칸이 쓰는 인증서 요약을 **보조 읽기**로 가져온다(서버 전용). 목록의 필수 읽기와
 * 따로 읽으므로, 못 읽으면 `null` 을 돌려주고 목록은 그대로 그린다 — TLS 칸이 「확인 불가」가 된다.
 *
 * - 로컬: 인증서 화면과 같은 뷰(`/certificates`).
 * - 노드: 그 노드의 인증서 화면 뷰(`/nodes/<id>/certificates`). 노드 목록 뷰가 무엇을 읽는지는 API 가
 *   정하므로(`lib/node-page.tsx`), 목록 뷰에 인증서를 끼워 넣지 않고 인증서 뷰를 한 번 더 읽는다.
 *   API 를 바꾸지 않는 대가로 노드에 요청이 한 번 더 간다.
 */

export async function readLocalCertificateSummaries(): Promise<CertificateSummary[] | null> {
  const view = await readOptionalView<CertificateListView>("/certificates");
  if (!view.ok) return null;
  try {
    return view.data.data.items.map((c) => ({ id: c.id, status: c.status, expiresAt: readInstant(c.expiresAt) }));
  } catch {
    return null;
  }
}

interface NodeCertificatesView {
  ok: boolean;
  data?: { certificates?: { certificates?: { id: string; status: string; expiresAt: string | null }[] } };
}

export async function readNodeCertificateSummaries(nodeId: string): Promise<CertificateSummary[] | null> {
  const view = await readOptionalView<NodeCertificatesView>(`/nodes/${encodeURIComponent(nodeId)}/certificates`);
  if (!view.ok || !view.data.ok) return null;
  const rows = view.data.data?.certificates?.certificates;
  if (!Array.isArray(rows)) return null;
  try {
    return rows.map((c) => ({ id: c.id, status: c.status, expiresAt: readInstant(c.expiresAt) }));
  } catch {
    // 날짜를 읽지 못하면(모양이 다른 노드) 추측하지 않고 「확인 불가」로 둔다.
    return null;
  }
}
