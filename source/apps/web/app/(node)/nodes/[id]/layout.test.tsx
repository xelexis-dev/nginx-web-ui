import { Children, isValidElement, type ReactNode } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { ContextBar } from "@repo/ui";
vi.mock("@/lib/api-client", () => ({ readSession: vi.fn(), ApiRequestError: class extends Error {} }));
vi.mock("@/lib/api-page", () => ({ readPageView: vi.fn() }));
vi.mock("@/lib/operations-shell-data", () => ({ readOperationsShellData: vi.fn() }));
import { readSession } from "@/lib/api-client";
import { readPageView } from "@/lib/api-page";
import { readOperationsShellData } from "@/lib/operations-shell-data";
import NodeLayout from "./layout";
beforeEach(() => vi.clearAllMocks());

it.each([["2.1.1", "2.1.1"], [null, "2.1.0"], [undefined, "2.1.0"]] as const)("uses live version %s with the stored version as fallback", async (liveVersion, expected) => {
  const storedVersion = "2.1.0";
  vi.mocked(readSession).mockResolvedValue({ user: { role: "admin", email: "admin@example.test" } } as Awaited<ReturnType<typeof readSession>>);
  vi.mocked(readPageView).mockImplementation(async () => ({ node: { name: "Node", lastStatus: "ok", lastVersion: storedVersion } }) as never);
  vi.mocked(readOperationsShellData).mockImplementation(async () => {
    return { available: true, nodeVersion: liveVersion } as Awaited<ReturnType<typeof readOperationsShellData>>;
  });
  const tree = await NodeLayout({ children: null, params: Promise.resolve({ id: "node" }) });
  let version: unknown;
  function visit(children: ReactNode) {
    Children.forEach(children, child => {
      if (!isValidElement<{ children?: ReactNode; contentHeader?: ReactNode; status?: ReactNode }>(child)) return;
      if (child.type === ContextBar && isValidElement<{ version?: string }>(child.props.status)) version = child.props.status.props.version;
      visit(child.props.children); visit(child.props.contentHeader);
    });
  }
  visit(tree);
  expect(version).toBe(expected);
  expect(readOperationsShellData).toHaveBeenCalledExactlyOnceWith("node");
  expect(readPageView).toHaveBeenCalledExactlyOnceWith("/nodes/node/layout");
});


it("starts the stored metadata read before a pending unavailable shell read settles", async () => {
  let settle!: (value: Awaited<ReturnType<typeof readOperationsShellData>>) => void;
  vi.mocked(readSession).mockResolvedValue({user: {role: "admin"}} as Awaited<ReturnType<typeof readSession>>);
  vi.mocked(readPageView).mockResolvedValue({node: {name: "Node", lastStatus: "unreachable", lastVersion: "2.1.0"}});
  vi.mocked(readOperationsShellData).mockImplementation(() => new Promise(resolve => {settle = resolve;}));
  const pending = NodeLayout({children: null, params: Promise.resolve({id: "stopped"})});
  await vi.waitFor(() => expect(readOperationsShellData).toHaveBeenCalledExactlyOnceWith("stopped"));
  expect(readPageView).toHaveBeenCalledExactlyOnceWith("/nodes/stopped/layout");
  settle({available: false});
  await pending;
});
