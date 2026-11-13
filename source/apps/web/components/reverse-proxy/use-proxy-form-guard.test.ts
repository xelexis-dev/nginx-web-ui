import { afterEach, expect, it, vi } from "vitest";
import { bindProxyFormGuard } from "./use-proxy-form-guard";

afterEach(() => vi.unstubAllGlobals());
function fixture(dirty = true, confirmed = false) {
  const handlers = new Map<string, (event: never) => void>();
  const navigationHandlers = new Map<string, (event: never) => void>();
  const navigation = { addEventListener: (n: string, fn: (e: never) => void) => navigationHandlers.set(n, fn), removeEventListener: vi.fn((name: string) => navigationHandlers.delete(name)) };
  class Element {
    href = "https://app.test/reverse-proxy";
    target = "";
    closest() { return this; }
    hasAttribute() { return false; }
  }
  const confirm = vi.fn(() => confirmed);
  const window = { location: new URL("https://app.test/reverse-proxy/s/edit"), navigation, confirm, addEventListener: (n: string, fn: (e: never) => void) => handlers.set(n, fn), removeEventListener: vi.fn((name: string) => handlers.delete(name)) };
  vi.stubGlobal("Element", Element);
  vi.stubGlobal("window", window);
  vi.stubGlobal("document", { addEventListener: window.addEventListener, removeEventListener: window.removeEventListener });
  const cleanup = bindProxyFormGuard(() => dirty);
  const event = { button: 0, target: new Element(), defaultPrevented: false, ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, preventDefault: vi.fn(), stopImmediatePropagation: vi.fn(), returnValue: undefined };
  return { window, event, cleanup, confirm, dispatch: (name: string) => handlers.get(name)?.(event as never), navigate: (destination: object, navigationType = "push", cancelable = true) => navigationHandlers.get("navigate")?.({ destination, navigationType, cancelable, preventDefault: event.preventDefault } as never) };
}
it("blocks dirty in-app links before Next handles them, keeping the draft on cancel", () => {
  const f = fixture(); f.dispatch("click");
  expect(f.confirm).toHaveBeenCalledOnce();
  expect(f.event.preventDefault).toHaveBeenCalledOnce();
  expect(f.event.stopImmediatePropagation).toHaveBeenCalledOnce();
});
it("asks the browser to protect a dirty tab close or reload", () => {
  const f = fixture(); f.dispatch("beforeunload");
  expect(f.event.preventDefault).toHaveBeenCalledOnce();
  expect(f.event.returnValue).toBe("");
});
it("cancels dirty same-document traversal before the router unmounts the draft", () => {
  const f = fixture();
  f.navigate({ url: f.event.target.href, sameDocument: true }, "traverse");
  expect(f.confirm).toHaveBeenCalledOnce();
  expect(f.event.preventDefault).toHaveBeenCalledOnce();
});
it("does not trap a non-cancelable browser traversal", () => {
  const f = fixture();
  f.navigate({ url: f.event.target.href, sameDocument: true }, "traverse", false);
  expect(f.confirm).not.toHaveBeenCalled();
  expect(f.event.preventDefault).not.toHaveBeenCalled();
});
it("does not ask twice for a confirmed same-origin full navigation", () => {
  const f = fixture(true, true); f.dispatch("click");
  f.navigate({ url: f.event.target.href, sameDocument: false });
  f.dispatch("beforeunload");
  expect(f.confirm).toHaveBeenCalledOnce();
  expect(f.event.preventDefault).not.toHaveBeenCalled();
});
it("leaves clean, new-tab and same-page links alone", () => {
  const clean = fixture(false); clean.dispatch("click"); clean.dispatch("beforeunload");
  expect(clean.confirm).not.toHaveBeenCalled();
  const modified = fixture(); modified.event.ctrlKey = true; modified.dispatch("click");
  expect(modified.confirm).not.toHaveBeenCalled();
  const same = fixture(); same.event.target.href = same.window.location.href + "#route"; same.dispatch("click");
  expect(same.confirm).not.toHaveBeenCalled();
});

it("removes all guards when the form leaves", () => {
  const f = fixture(); f.cleanup();
  f.dispatch("click"); f.dispatch("beforeunload");
  expect(f.confirm).not.toHaveBeenCalled();
  expect(f.event.preventDefault).not.toHaveBeenCalled();
});
