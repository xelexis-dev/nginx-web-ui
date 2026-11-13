"use client";

import { useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { Banner, Button } from "@repo/ui";
// 라우트 그룹의 시트를 그대로 들여온다 — 왜 복사하지 않는지는
// `certificate-list.tsx` 헤더(요약: `composes` 가 걸려 있고 로컬에 남는 형제
// 화면들이 같은 클래스를 쓴다).
import styles from "@/app/(app)/certificates/certificates.module.css";

/**
 * 진행 중인 발급·갱신 중단 버튼. 로컬 상세와 노드 상세가 같은 것을 쓴다.
 *
 * 서버 액션이 아니라 `fetch`로 Route Handler를 부른다 — Next.js는 서버 액션을
 * 클라이언트당 직렬로 dispatch하므로, "지금 갱신"이 대기 중일 때 서버 액션 중단은
 * 그 뒤에 줄을 서서 아무것도 끊지 못한다. **노드에서는 더 강하게 성립한다**:
 * 노드의 갱신 라우트가 최대 300초를 붙들고 있어(`maxDuration`) 게이트웨이의 갱신
 * 액션도 그동안 살아 있다.
 *
 * **경로를 props 로 받는 것이 요점이다** — 계획 4a 의 `MetricsView.dataPath`·
 * `LogViewer.streamPath` 와 같은 모양이다. 로컬은
 * `/api/certificates/<id>/cancel`, 노드는 중계 라우트
 * `/api/nodes/<nodeId>/certificates/<cid>/cancel` 을 넘긴다. 여기서 `id` 로부터
 * 경로를 조립하면 노드 화면의 중단이 **게이트웨이 자신의 발급**을 끊으러 간다 —
 * 버튼은 멀쩡히 눌리고 아무 오류도 안 난다.
 *
 */
export function CancelIssuanceButton({
  cancelPath,
  label = "발급 중단",
}: {
  /** `POST` 할 중단 경로. 로컬/노드가 서로 다른 라우트 핸들러를 가리킨다. */
  cancelPath: string;
  label?: string;
}) {
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function cancel() {
    setBusy(true);
    setError(null);
    try {
      const res = await fetch(cancelPath, { method: "POST" });
      if (res.status === 401) {
        router.push("/login");
        return;
      }
      if (!res.ok) {
        const body = (await res.json().catch(() => null)) as { error?: unknown } | null;
        const error = body?.error;
        setError(typeof error === "string" ? error
          : error && typeof error === "object" && "message" in error && typeof error.message === "string"
            ? error.message : `중단하지 못했습니다 (HTTP ${res.status})`);
        return;
      }
      startTransition(() => router.refresh());
    } catch (err) {
      setError(err instanceof Error ? err.message : "중단 요청을 보내지 못했습니다.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className={styles.stack}>
      {/* danger(빨강)가 아니라 secondary다 — 바로 옆 "삭제"와 같은 빨간 버튼 두 개가
          나란히 서면 오클릭을 부른다. 중단은 되돌릴 수 있고(다시 발급) 삭제는 아니다. */}
      <Button type="button" variant="secondary" onClick={cancel} disabled={busy || pending}>
        {busy ? "중단 중..." : label}
      </Button>
      {error ? (
        <Banner tone="error" title={error} />
      ) : null}
    </div>
  );
}
