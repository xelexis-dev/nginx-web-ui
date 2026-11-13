"use client";
import { useEffect, useRef, type RefObject } from "react";

const LEAVE_MESSAGE = "저장하지 않은 변경 사항이 있습니다. 페이지를 떠나시겠습니까?";
type FormNavigateEvent = Event & {
  navigationType?: string;
  destination?: { url: string; sameDocument: boolean };
};

export function bindProxyFormGuard(isDirty: () => boolean) {
  const navigation = (window as unknown as { navigation?: EventTarget }).navigation;
  let confirmedHref: string | undefined;
  let approvedUnload = false;

  function onClick(event: MouseEvent) {
    if (event.defaultPrevented || event.button !== 0 ||
        event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    const anchor = event.target instanceof Element ? event.target.closest<HTMLAnchorElement>("a[href]") : null;
    if (!anchor || anchor.hasAttribute("download") || (anchor.target && anchor.target !== "_self")) return;
    const target = new URL(anchor.href, window.location.href);
    if (target.origin !== window.location.origin ||
        target.pathname === window.location.pathname || !isDirty()) return;
    if (window.confirm(LEAVE_MESSAGE)) confirmedHref = target.href;
    else {
      confirmedHref = undefined;
      event.preventDefault();
      event.stopImmediatePropagation();
    }
  }
  function onBeforeUnload(event: BeforeUnloadEvent) {
    if (approvedUnload) { approvedUnload = false; return; }
    if (isDirty()) {
      event.preventDefault();
      event.returnValue = "";
    }
  }
  function onNavigate(event: Event) {
    const change = event as FormNavigateEvent;
    const destination = change.destination;
    // shortcut: traversal needs Navigation API support; respect the browser’s non-cancelable escape.
    if (change.navigationType === "traverse" &&
        destination?.sameDocument && event.cancelable &&
        new URL(destination.url).pathname !== window.location.pathname && isDirty() &&
        !window.confirm(LEAVE_MESSAGE)) event.preventDefault();
    if (destination && destination.url === confirmedHref) {
      approvedUnload = !destination.sameDocument;
      confirmedHref = undefined;
    }
  }
  function onNavigateError() {
    approvedUnload = false;
    confirmedHref = undefined;
  }
  document.addEventListener("click", onClick, true);
  window.addEventListener("beforeunload", onBeforeUnload);
  navigation?.addEventListener("navigate", onNavigate);
  navigation?.addEventListener("navigateerror", onNavigateError);
  return () => {
    document.removeEventListener("click", onClick, true);
    window.removeEventListener("beforeunload", onBeforeUnload);
    navigation?.removeEventListener("navigate", onNavigate);
    navigation?.removeEventListener("navigateerror", onNavigateError);
  };
}

const snapshot = (data: FormData) => JSON.stringify([...data.entries()]);
export function useProxyFormGuard(form: RefObject<HTMLFormElement | null>, pending: boolean) {
  const baseline = useRef<string | undefined>(undefined);
  const dirty = useRef(false);
  const saved = useRef(false);
  useEffect(() => {
    if (!form.current || pending) return;
    const current = snapshot(new FormData(form.current));
    if (baseline.current === undefined || saved.current) baseline.current = current;
    dirty.current = current !== baseline.current;
    saved.current = false;
  }, [form, pending]);
  useEffect(() => bindProxyFormGuard(() => {
    if (!form.current || baseline.current === undefined || saved.current) return false;
    if (form.current.getAttribute("aria-busy") === "true") return dirty.current;
    return snapshot(new FormData(form.current)) !== baseline.current;
  }), [form]);
  return {
    begin(data: FormData) {
      dirty.current = snapshot(data) !== baseline.current;
    },
    finish(ok: boolean) {
      saved.current = ok;
      if (ok) dirty.current = false;
    },
  };
}
