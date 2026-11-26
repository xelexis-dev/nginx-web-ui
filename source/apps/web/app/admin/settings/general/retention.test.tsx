import { Children, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { RetentionCollectionView, RetentionSettingsView } from "@repo/contracts";
import { AuditList } from "@/components/audit/audit-list";
import { HistoryList } from "@/components/history/history-list";
import { RetentionForm, needsRetentionConfirmation } from "./retention-form";

const hooks = vi.hoisted(() => ({ state: [] as unknown[], cursor: 0, active: false }));
vi.mock("react", async (original) => {
  const react = await original<typeof import("react")>();
  return {
    ...react,
    useState<T>(initial: T | (() => T)) {
      if (!hooks.active) return react.useState(initial);
      const i = hooks.cursor++;
      if (!(i in hooks.state)) hooks.state[i] = typeof initial === "function" ? (initial as () => T)() : initial;
      return [hooks.state[i] as T, (value: T | ((previous: T) => T)) => {
        hooks.state[i] = typeof value === "function" ? (value as (previous: T) => T)(hooks.state[i] as T) : value;
      }] as const;
    },
  };
});

vi.mock("next/navigation", () => ({
  useRouter: () => ({ refresh: vi.fn() }),
  usePathname: () => "/audit",
}));

const at = "2026-11-20T06:24:10Z";
const off: RetentionCollectionView = {
  days: null,
  count: 412,
  oldestAt: null,
  lastCleanup: null,
  nextRunAt: null,
  remaining: 0,
  lastFailure: null,
  status: "off",
};
const cleaned = { at, deleted: 20_000 };

describe("record retention settings", () => {
  it.each([
    [null, null, false],
    [null, 90, true],
    [365, 90, true],
    [90, 365, false],
    [90, 90, false],
    [90, null, false],
  ])("confirms proposed limits only on enabling or shortening (%s → %s)", (before, after, confirms) => {
    expect(needsRetentionConfirmation(before, after)).toBe(confirms);
  });

  it.each([
    ["off", "꺼짐", "없음"],
    ["waiting", "첫 정리 대기", "약 10분 뒤"],
    ["running", "정리 중", "2,000건 남음"],
    ["done", "정리 완료", "지울 기록이 있을 때"],
    ["failed", "정리 실패", "다시 시도"],
  ] as const)("renders status and next cleanup for %s", (status, label, next) => {
    const audit: RetentionCollectionView = {
      ...off, status, days: status === "off" ? null : 90, nextRunAt: at,
      remaining: 2_000, lastCleanup: cleaned, lastFailure: { at, deleted: 25, message: "fixture cleanup error" },
    };
    const html = renderToStaticMarkup(<RetentionForm values={{ audit, history: off }} />);
    expect(html).toContain(label);
    expect(html).toContain(next);
    expect(html).toContain("20,000건");
    expect(html).toContain("한 번도 없음");
    expect(html.includes("fixture cleanup error")).toBe(status === "failed");
  });
});

function renderForm(values: RetentionSettingsView) {
  hooks.cursor = 0;
  hooks.active = true;
  try {
    return RetentionForm({ values });
  } finally {
    hooks.active = false;
  }
}

function findInput(node: ReactNode, name: string): { value: string; onChange: (event: { target: { value: string } }) => void } | undefined {
  if (!isValidElement<{ children?: ReactNode; name?: string; value: string; onChange: (event: { target: { value: string } }) => void }>(node)) return;
  if (node.props.name === name) return node.props;
  return Children.toArray(node.props.children).map((child) => findInput(child, name)).find(Boolean);
}

it.each([false, true])("binds submissions to the right policies after a refresh (dirty = %s)", async (dirty) => {
  hooks.state = [];
  const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(dirty
    ? new Response(JSON.stringify({ error: { code: "conflict", message: "changed" } }), { status: 409 })
    : new Response(JSON.stringify({ schedulesCleanup: false })));
  vi.stubGlobal("fetch", fetch);
  try {
    const audit = { ...off, days: 90, status: "done" } as const;
    const initial = renderForm({ audit, history: off });
    if (dirty) findInput(initial, "auditRetentionDays")!.onChange({ target: { value: "120" } });

    const props = { audit, history: { ...off, days: 365, status: "done" } as const };
    const refreshed = renderForm(props);
    expect(findInput(refreshed, "historyRetentionDays")!.value).toBe(dirty ? "" : "365");
    findInput(refreshed, "auditRetentionDays")!.onChange({ target: { value: "150" } });
    const edited = renderForm(props);
    await edited.props.onSubmit({ preventDefault() {} });

    expect(fetch).toHaveBeenCalledTimes(1);
    const body = fetch.mock.lastCall?.[1]?.body;
    if (!(body instanceof URLSearchParams)) throw new Error("Expected URL-encoded retention form");
    expect(body.get("auditRetentionDays")).toBe("150");
    expect(body.get("expectedAuditRetentionDays")).toBe("90");
    expect(body.get("historyRetentionDays")).toBe(dirty ? "" : "365");
    expect(body.get("expectedHistoryRetentionDays")).toBe(dirty ? "" : "365");
    if (dirty) expect(JSON.stringify(renderForm(props))).toContain("changed");
  } finally {
    vi.unstubAllGlobals();
    hooks.state = [];
  }
});

it("keeps a saved disable visible and authoritative until refreshed props arrive", async () => {
  hooks.state = [];
  const fetch = vi.fn<typeof globalThis.fetch>().mockImplementation(async () =>
    new Response(JSON.stringify({ schedulesCleanup: false })),
  );
  vi.stubGlobal("fetch", fetch);
  try {
    const oldProps = { audit: { ...off, days: 90, status: "done" } as const, history: off };
    const initial = renderForm(oldProps);
    findInput(initial, "auditRetentionDays")!.onChange({ target: { value: "" } });
    await renderForm(oldProps).props.onSubmit({ preventDefault() {} });

    const acknowledged = renderForm(oldProps);
    expect(findInput(acknowledged, "auditRetentionDays")!.value).toBe("");
    expect(JSON.stringify(acknowledged)).toContain("보존 기간을 저장했습니다.");
    await acknowledged.props.onSubmit({ preventDefault() {} });
    const body = fetch.mock.lastCall?.[1]?.body;
    if (!(body instanceof URLSearchParams)) throw new Error("Expected URL-encoded retention form");
    expect(body.get("expectedAuditRetentionDays")).toBe("");

    const refreshed = renderForm({ audit: off, history: off });
    expect(findInput(refreshed, "auditRetentionDays")!.value).toBe("");
    const changedElsewhere = renderForm({ audit: { ...off, days: 180, status: "done" }, history: off });
    expect(findInput(changedElsewhere, "auditRetentionDays")!.value).toBe("180");
  } finally {
    vi.unstubAllGlobals();
    hooks.state = [];
  }
});

it("keeps a newer policy observed before a delayed save acknowledgement", async () => {
  hooks.state = [];
  const success = () => new Response(JSON.stringify({ schedulesCleanup: false }));
  let acknowledge!: (response: Response) => void;
  const delayed = new Promise<Response>((resolve) => { acknowledge = resolve; });
  const fetch = vi.fn<typeof globalThis.fetch>()
    .mockReturnValueOnce(delayed)
    .mockImplementation(async () => success());
  vi.stubGlobal("fetch", fetch);
  try {
    const oldProps = { audit: { ...off, days: 90, status: "done" } as const, history: off };
    const initial = renderForm(oldProps);
    findInput(initial, "auditRetentionDays")!.onChange({ target: { value: "" } });
    const saving = renderForm(oldProps).props.onSubmit({ preventDefault() {} });

    const refreshedProps = { audit: { ...off, days: 180, status: "done" } as const, history: off };
    renderForm(refreshedProps);
    acknowledge(success());
    await saving;

    const refreshed = renderForm(refreshedProps);
    expect(findInput(refreshed, "auditRetentionDays")!.value).toBe("180");
    await refreshed.props.onSubmit({ preventDefault() {} });
    expect(fetch).toHaveBeenCalledTimes(2);
    const body = fetch.mock.lastCall?.[1]?.body;
    if (!(body instanceof URLSearchParams)) throw new Error("Expected URL-encoded retention form");
    expect(body.get("expectedAuditRetentionDays")).toBe("180");
  } finally {
    vi.unstubAllGlobals();
    hooks.state = [];
  }
});

describe("retention cleanup end notes", () => {
  it("shows the history note only after the first cleanup", () => {
    const list = (lastCleanup?: typeof cleaned) =>
      renderToStaticMarkup(<HistoryList info="" data={{ items: [], showDetails: true, lastCleanup }} />);
    expect(list()).not.toContain("목록은 최근 50건까지 보여 줍니다.");
    expect(list(cleaned)).toContain("목록은 최근 50건까지 보여 줍니다.");
  });

  it("shows the cleanup note when a filtered last page becomes page one", () => {
    const html = renderToStaticMarkup(<AuditList basePath="/audit" data={{
      items: [{ id: "kept", time: "2026-11-20", action: "test:filtered", target: "kept", actorEmail: null, actorLabel: null }],
      actions: ["test:filtered"], actionFilter: "test:filtered", page: 1, totalPages: 1, total: 50, lastCleanup: cleaned,
    }} />);
    expect(html).toContain("1 / 1");
    expect(html).toContain("오래된 기록 가운데 일부는 보존 기간이 지나 지워졌습니다.");
  });

  it.each(["", "retention:prune"])("shows UTC audit cleanup on the final page with filter %s", (actionFilter) => {
    const list = (page: number) => renderToStaticMarkup(<AuditList basePath="/audit" data={{
      items: [], actions: ["retention:prune"], actionFilter, page, totalPages: 2, total: 51, lastCleanup: cleaned,
    }} />);
    expect(list(1)).not.toContain("오래된 기록 가운데 일부는 보존 기간이 지나 지워졌습니다.");
    expect(list(2)).toMatch(/오래된 기록 가운데 일부는 보존 기간이 지나 지워졌습니다\..*UTC · 20,000건/);
  });
});
