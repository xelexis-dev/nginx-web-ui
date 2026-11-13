/** Poll while visible, and refresh once when the tab becomes visible again. */
export function startVisibleInterval(run: () => void, ms: number): () => void {
  const tick = () => { if (document.visibilityState === "visible") run(); };
  const timer = setInterval(tick, ms);
  document.addEventListener("visibilitychange", tick);
  return () => { clearInterval(timer); document.removeEventListener("visibilitychange", tick); };
}
