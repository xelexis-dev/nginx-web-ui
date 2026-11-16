import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, expect, it, vi } from "vitest";
import type { ApplyBarResult } from "./apply-bar";
import { applyResultToast } from "./apply-bar";
import { ReviewResultToast } from "./apply-dialog";
import { NodeErrorCard, type NodeErrorCardProps } from "./node-error";

const hooks = vi.hoisted(() => ({
  result: null as ApplyBarResult | null,
  ref: { current: null as ApplyBarResult | null },
  toast: vi.fn(),
}));
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useActionState: () => [hooks.result, () => {}, false],
  useRef: () => hooks.ref,
  useEffect: (fn: () => void) => fn(),
}));
vi.mock("./notifications/toast-provider", () => ({
  useToast: () => ({ toast: hooks.toast }),
}));
vi.mock("@/lib/use-api-action", () => ({
  useApiAction: (action: unknown) => action,
}));
beforeEach(() => {
  hooks.result = null;
  hooks.ref.current = null;
  hooks.toast.mockClear();
});
const output = "nginx: [emerg] bad directive\nnginx: test failed";
it.each([
  [{ ok: true, output }, "success", "nginx -t 통과, reload 완료", false],
  [
    {
      ok: true,
      output:
        "nginx: test is successful\n\n새 변경을 적용하지 못한 설정 1개:\n- 리버스 프록시 a.test: nginx -t 실패",
    },
    "warning",
    "일부 설정을 적용하지 못했습니다",
    true,
  ],
  [
    {
      ok: true,
      output:
        "nginx: test is successful\n\n적용에서 제외한 설정 2개 — 이 항목들은 지금 동작하지 않습니다:\n- stream 1: 포트 충돌",
    },
    "warning",
    "일부 설정을 적용하지 못했습니다",
    true,
  ],
  [
    { ok: false, output },
    "error",
    "적용 실패 — nginx -t 실패 (변경 사항은 롤백됨)",
    true,
  ],
  [{ ok: false, error: "연결 실패" }, "error", "연결 실패", true],
] as const)(
  "maps result %j to a toast",
  (result, severity, title, persistent) => {
    expect(applyResultToast(result)).toMatchObject({
      key: "apply-result",
      severity,
      title,
      persistent,
    });
    expect(applyResultToast(result).output).toBe(
      "output" in result ? result.output : undefined,
    );
  },
);
const failures: NodeErrorCardProps["failure"][] = [
  { kind: "unreachable", message: output },
  { kind: "unauthorized", message: "끊김" },
  { kind: "forbidden", message: "권한 없음" },
  { kind: "version_mismatch", message: "버전", nodeVersion: "1.0.0" },
  { kind: "too_old", message: "이전 버전" },
  { kind: "not_found", message: "없음" },
  { kind: "node_error", message: "오류" },
  { kind: "conflict", message: "충돌" },
  { kind: "rate_limited", message: "너무 많은 요청" },
  { kind: "domain_failure", message: output },
];
it.each(failures)("shares card title/body/action for $kind", (failure) => {
  const nodeError = {
    failure,
    nodeId: "n",
    retryHref: "/retry",
    listHref: "/list",
  };
  const toast = applyResultToast({ ok: false, nodeError });
  const html = renderToStaticMarkup(<NodeErrorCard {...nodeError} />);
  expect(html).toContain(toast.title);
  if (toast.body) expect(html).toContain(toast.body);
  if (failure.kind === "domain_failure") {
    expect(toast.output).toBe(failure.message);
    expect(toast.body).toBeUndefined();
  } else expect(toast.output).toBeUndefined();
  if (toast.link) {
    expect(html).toContain(`href="${toast.link}"`);
    expect(html).toContain(toast.linkLabel);
  }
});
it("emits once on rerender/reopen, closes only success, and emits a new submission", () => {
  const close = vi.fn();
  const renderResult = () => {
    ReviewResultToast({ result: hooks.result, onSuccess: close });
  };
  renderResult();
  expect(hooks.toast).not.toHaveBeenCalled();
  hooks.result = { ok: false, output };
  renderResult();
  renderResult();
  expect(hooks.toast).toHaveBeenCalledTimes(1);
  expect(close).not.toHaveBeenCalled();
  hooks.result = { ok: false, output };
  renderResult();
  expect(hooks.toast).toHaveBeenCalledTimes(2);
  hooks.result = { ok: true };
  renderResult();
  renderResult();
  expect(hooks.toast).toHaveBeenCalledTimes(3);
  expect(close).toHaveBeenCalledTimes(1);
});

it("does not claim rollback for a node validation error", () => {
  const toast = applyResultToast({
    ok: false,
    nodeError: {
      nodeId: "n",
      retryHref: "/retry",
      listHref: "/list",
      failure: { kind: "validation", message: "입력 오류", fieldErrors: {} },
    },
  });
  expect(toast.title).toBe("입력 오류");
});
