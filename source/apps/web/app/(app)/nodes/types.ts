/**
 * 이 기능의 서버 액션 결과 모양. `"use server"` 가 **없는** 모듈에 두는 것이
 * 요점이다 — 그래야 `actions.ts` 와 클라이언트 폼 컴포넌트가 서로를 끌어들이지
 * 않고 이 타입만 가져갈 수 있다.
 */
export interface ActionState {
  ok: boolean;
  error?: string;
  fieldErrors?: Record<string, string>;
  /**
   * **몇 번째 제출인가.** 화면이 실패를 *다시* 알려야 하는지를 판정하는 값이다 —
   * 문구가 아니라 이 숫자가 매 제출마다 달라진다.
   *
   * 없으면 **같은 사유로 두 번 실패할 때 화면이 조용해진다.** 오류 문구를
   * 의존성으로 쓰면 두 번째 실패에서 `Object.is` 가 성립해 `useEffect` 가 다시
   * 돌지 않고(배너로 스크롤하지 않고), `role="alert"` 도 DOM 텍스트가 그대로면
   * 다시 읽어 주지 않는다. 틀린 토큰을 붙여넣고 또 붙여넣는 것이 이 화면에서
   * 가장 흔한 경로라, 하필 거기서 아무 말도 없게 된다.
   *
   * `pairNode` 만 채운다(`useActionState` 가 넘겨 주는 이전 상태에서 +1).
   *
   * ⚠ **`updateNode`·`unpairNode` 는 같은 `ActionState` 를 쓰면서 이 값을 세지 않는다.**
   * 지금은 그 둘을 `useActionState` 로 쓰는 폼이 없어 아무 일도 일어나지 않는다.
   * 하지만 그런 폼을 만들면서 이 배너 관용(`key={state.attempt}` + `[state.attempt]`
   * 의존성)을 함께 베끼면, `attempt` 가 `undefined` 로 고정돼 **두 번째 실패부터
   * 화면이 조용해진다** — 검사는 전부 통과한 채로. 그때는 그 액션에도 `attempt` 를
   * 이어 세는 것을 함께 넣어라.
   */
  attempt?: number;
}
