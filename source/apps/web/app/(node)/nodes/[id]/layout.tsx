import shellStyles from "@/components/operations/operations-shell.module.css";
import { readOperationsShellData } from "@/lib/operations-shell-data";
import { ContextBar, StatusBadge } from "@repo/ui";
import Link from "next/link";
import { NodeStatusBadge } from "@/components/node-error";
import {
  OperationsSectionNav,
  OperationsShell,
} from "@/components/operations/operations-shell";
import type { ReactNode } from "react";
import { redirect, unstable_rethrow } from "next/navigation";
import { ErrorView } from "@/components/error-view";
import ErrorBoundary from "@/components/error-boundary";
import { ToastProvider } from "@/components/notifications/toast-provider";
import { ApiRequestError, readSession } from "@/lib/api-client";
import { readPageView } from "@/lib/api-page";
import type { NodeView } from "@/lib/node-page";
import { SessionActivity } from "@/components/session-activity";
import { nodeNavCategories } from "./node-nav";

// 노드 맥락 셸: `(app)/layout.tsx` 와 같은 뼈대(ToastProvider / topbar /
// ErrorBoundary / .content)를 쓰되 사이드바만 이 노드의 메뉴로 바꾼다.
// `/admin` 이 같은 이유로 `(app)` 의 형제인 것과 같다 — 중첩 레이아웃은 부모의
// 사이드바를 대체할 수 없고, 서버 레이아웃은 pathname 을 모른다.
//
// **`(node)/layout.tsx` 가 아니라 여기(`nodes/[id]/layout.tsx`)에 둔다.** Next.js
// 레이아웃은 "root 세그먼트부터 그 레이아웃 자리까지"의 `params`만 받는다 —
// `[id]` 보다 위(`(node)/layout.tsx`)에 두면 `params`가 비어 있어 아래 `id` 조회가
// 안 된다. 노드 화면은 모두 `nodes/[id]/**` 아래에 두므로 여기서 노드 맥락을
// 구성해도 형제 셸 구조와 URL은 유지된다.
export const runtime = "nodejs";

export default async function NodeLayout({
  children,
  crumbs,
  params,
}: {
  children: ReactNode;
  crumbs?: ReactNode;
  params: Promise<{ id: string }>;
}) {
  // 세션 확인은 DB 를 친다 — DB 가 죽으면 여기서 던진다. 경계에 기대지 않고
  // 직접 받는 이유는 `(app)/layout.tsx` 의 같은 주석 참고(여기서도 동일하다).
  let session: Awaited<ReturnType<typeof readSession>>;
  try {
    session = await readSession();
  } catch (err) {
    unstable_rethrow(err);
    if (err instanceof ApiRequestError && err.status === 401)
      redirect("/login");
    console.error("[node-shell] 세션 확인 실패:", err);
    return <ErrorView standalone homeHref="/login" homeLabel="로그인으로" />;
  }

  const { id } = await params;

  // 이름은 로컬 metadata, 상태 띠는 해당 노드의 기존 RP view에서 읽는다.
  // 상태 읽기가 실패해도 페이지 자체의 상세 오류 처리는 유지한다.
  const { node } = await readPageView<{ node: NodeView }>(
    `/nodes/${id}/layout`,
  );

  const shellData = await readOperationsShellData(id);
  return (
    <ToastProvider>
      <SessionActivity />
      <OperationsShell
        role={session.user.role}
        email={session.user.email ?? ""}
        target={{ kind: "node", id, name: node.name }}
        data={shellData}
        crumbs={crumbs}
        contentHeader={
          <>
            <div className={shellStyles.nodeContextBand}>
              <div className={shellStyles.shellColumn}>
                <ContextBar
                  name={node.name}
                  offline={!shellData.available}
                  status={
                    shellData.available ? (
                      <NodeStatusBadge
                        status={node.lastStatus}
                        version={node.lastVersion}
                      />
                    ) : (
                      <StatusBadge tone="error" size="sm">
                        조회할 수 없음
                      </StatusBadge>
                    )
                  }
                  renderLink={(link) => <Link {...link} />}
                />
              </div>
            </div>
            <div className={shellStyles.shellColumn}>
              <OperationsSectionNav
                items={nodeNavCategories(id).flatMap((group) => group.items)}
                variant="tabs"
                label="노드 메뉴"
              />
            </div>
          </>
        }
      >
        <ErrorBoundary homeHref="/nodes" homeLabel="노드 목록으로">
          {children}
        </ErrorBoundary>
      </OperationsShell>
    </ToastProvider>
  );
}
