/**
 * Presentation-only result types shared by local and remote API forms.
 * This module has no runtime imports or backend execution dependencies.
 */
import type { NodeErrorCardProps } from "@/components/node-error";
import type { ActionRef } from "@repo/contracts";

export interface NodeActionState {
  ok: boolean;
  /**
   * 사람이 읽을 한 줄. 실패면 노드가 준 `message` 가 그대로 온다.
   * `nodeError` 를 아직 그리지 않는 폼도 최소한 이건 보여준다.
   */
  error?: string;
  /** `kind === "validation"` 일 때만. 키는 **폼 필드 이름**이다(스펙 §5). */
  fieldErrors?: Record<string, string>;
  /**
   * `validation` **이 아닌** 실패 — `NodeErrorCard` 에 그대로 펼치면 되는 한
   * 덩어리다(`<NodeErrorCard {...state.nodeError} />`).
   *
   * **`NodeFailure` 만 담지 않는 이유:** 그 카드는 `failure` 외에 `nodeId` 와
   * `retryHref` 를 요구한다. 그 둘을 폼의 **별도 optional prop** 으로 받게 하면
   * 노드 화면이 하나만 빠뜨려도 타입 오류 없이 카드가 사라진다 — 이 헬퍼가
   * 막으려는 실수와 정확히 같은 모양이다. 실패와 함께 한 덩어리로 실어
   * 보내면 그럴 자리가 없다.
   *
   * **`fieldErrors` 와 동시에 채워지지 않는다.** `validation` 은 폼 필드로
   * 가고(스펙 §5), 나머지는 카드로 간다.
   */
  nodeError?: NodeErrorCardProps;
}

/**
 * 공유 폼 컴포넌트의 `action` prop 타입. **`ActionState` 로 선언하지 마라** —
 * 추가 필드가 전부 optional 이라 양방향 대입이 성립해서 컴파일은 통과하는데,
 * 컴포넌트 안에서 `state.nodeError` 가 타입에 없어 §9 카드가 영영 안 그려진다
 * (노드가 죽어도 사용자는 빨간 한 줄만 본다).
 *
 * **로컬 액션(`(prev: ActionState, fd) => Promise<ActionState>`)은 한 글자도
 * 고치지 않고 그대로 들어간다** — 파라미터는 반공변(`NodeActionState` →
 * `ActionState` 성립), 반환은 공변(`ActionState` → `NodeActionState` 성립)이라
 * 양쪽이 다 맞는다. 로컬 액션은 `nodeError` 를 채우는 일이 없으므로 로컬 화면
 * 동작도 그대로다.
 */
export type NodeFormAction = ActionRef | ((state: NodeActionState, formData: FormData) => Promise<NodeActionState>);

/**
 * **노드 서버 액션의 반환 타입은 이것으로 적는다.**
 *
 * ```ts
 * export async function createNodeUpstream(
 *   nodeId: string, _prev: NodeActionState, formData: FormData,
 * ): Promise<NodeFormActionResult> { … }
 * ```
 *
 * `NodeActionState` 의 별칭일 뿐이고 **일부러 그렇다** — 공칭(nominal) 브랜드로
 * 막으면 로컬 액션(`(prev: ActionState, fd) => Promise<ActionState>`)이 공유 폼의
 * prop 에 못 들어가고, 이 설계 전체가 그 호환 위에 서 있다.
 *
 * **⚠ 반환 타입을 `Promise<ActionState>` 로 적지 마라.** 바로 옆의 로컬 액션이
 * 그렇게 생겼고 그게 이 저장소의 관용이라 베끼기 쉬운데, `NodeActionState` 는
 * `ActionState` 에 대입되므로 **컴파일이 조용히 통과하고 `nodeError` 가 폼에
 * 닿기 전에 지워진다.** 그러면 노드가 죽어도 §9 카드가 안 나온다 — 폼 prop
 * 타입에서 고친 것과 **같은 결함이 한 층 위에서** 되살아난다. 타입 오류는
 * 어디에도 안 나므로 이 이름을 쓰는 것 말고는 막을 방법이 없다.
 */
export type NodeFormActionResult = NodeActionState;
