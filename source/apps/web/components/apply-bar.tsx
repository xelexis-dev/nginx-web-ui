"use client";

import type { ConfigDiff } from "@/lib/config-diff";
import type { OperationsTarget } from "@repo/presentation/operations-target";
import type { ActionRef } from "@repo/contracts";
import { Banner } from "@repo/ui";
import {
  NodeErrorCard,
  nodeErrorPresentation,
  type NodeErrorCardProps,
} from "@/components/node-error";
import type { ToastInput } from "./notifications/toast-provider";
export type { OperationsTarget } from "@repo/presentation/operations-target";

interface ReviewContext {
  target: OperationsTarget;
  targetLabel: string;
  onCancel: () => void;
}

/** One apply outcome. A node failure travels whole in `nodeError`, so the toast can use the shared node-error wording. */
export interface ApplyBarResult {
  /** `nginx -t` 통과 + reload 성공. */
  ok: boolean;
  /** nginx 가 낸 출력 그대로. 봉투에 `output` 필드가 따로 없는 노드 실패 경로는 이걸 쓰지 않는다. */
  output?: string;
  /** Browser API transport or request failure, before nginx reports a result. */
  error?: string;
  /**
   * `nginx -t` 실패를 **포함한** §9 실패 한 덩어리. `NodeErrorCard` 에 그대로
   * 펼친다. 로컬 액션은 절대 채우지 않는다.
   */
  nodeError?: NodeErrorCardProps;
}

/**
 * 적용 액션. **인자를 받지 않는다** — `useActionState` 는 항상 `(state, formData)`
 * 로 부르지만, 파라미터가 더 적은 함수는 그 타입에 구조적으로 대입되므로 로컬
 * `applyConfig()` 도 노드 `applyNodeConfig.bind(null, nodeId)` 도 그대로 들어간다.
 */
export type ApplyBarAction = ActionRef | (() => Promise<ApplyBarResult>);

export interface ApplyBarProps {
  /** `nginx:reload` 등급. 없으면 바 전체를 그리지 않는다(viewer). */
  canApply: boolean;
  /**
   * 미적용 변경 수. **노드 화면에서는 반드시 노드의 `/api/v1/pending` 값이다** —
   * 게이트웨이 자기 것을 넘기면 「적용할 게 없는데 배지가 떠 있다」가 된다.
   */
  pendingCount: number;
  pendingHasBaseline: boolean;
  /** 배지 모달의 내용. `pendingCount` 와 **같은 계산**에서 온 것이어야 어긋나지 않는다. */
  pendingDiff: ConfigDiff;
  applyAction: ApplyBarAction;
  review: ReviewContext;
}

export type ApplyReviewProps = Omit<ApplyBarProps, "review"> & ReviewContext;

/** Banner form of a result. The product shows results as a toast (`applyResultToast`); Storybook templates still render this. */
export function ApplyResult({ result }: { result: ApplyBarResult | null }) {
  if (!result) return null;

  // **노드 실패가 먼저다.** `nodeError` 가 있으면 §9 가 배너(422)와 카드(닿지
  // 못함)를 갈라 그린다 — 아래 로컬 배너로 보내면 「서버가 죽었다」가
  // 「변경 사항은 롤백됨」이라고 말하게 되는데, 그건 게이트웨이가 **하지도
  // 관측하지도 않은** 롤백을 단언하는 것이다.
  if (result.nodeError) return <NodeErrorCard {...result.nodeError} />;

  return (
    <Banner
      tone={result.ok ? "success" : "error"}
      title={
        <>
          {result.ok
            ? "nginx -t 통과, reload 완료"
            : (result.error ??
              "적용 실패 — nginx -t 실패 (변경 사항은 롤백됨)")}
        </>
      }
      role="status"
      output={result.output}
    ></Banner>
  );
}

// shortcut: matches the headings of Rust control/model.rs `skipped_summary`; use a structured field when the apply response next changes.
const PARTIAL_APPLY =
  /(새 변경을 적용하지 못한 설정|적용에서 제외한 설정) \d+개/;

export function applyResultToast(result: ApplyBarResult): ToastInput {
  const node = result.nodeError
    ? (nodeErrorPresentation(result.nodeError) ?? {
        title: result.nodeError.failure.message,
      })
    : null;
  // Apply leaves out what nginx -t rejects and reports ok: the rest is live, this part is not.
  const content =
    node && result.nodeError?.failure.kind === "domain_failure"
      ? { ...node, output: node.body, body: undefined }
      : node;
  const partial = result.ok && PARTIAL_APPLY.test(result.output ?? "");
  return {
    key: "apply-result",
    severity: partial ? "warning" : result.ok ? "success" : "error",
    persistent: partial || !result.ok,
    ...(content ?? {
      title: partial
        ? "일부 설정을 적용하지 못했습니다"
        : result.ok
          ? "nginx -t 통과, reload 완료"
          : (result.error ?? "적용 실패 — nginx -t 실패 (변경 사항은 롤백됨)"),
      output: result.output,
    }),
  };
}
