/** Explicit UTC keeps SSR and hydration consistent regardless of the process time zone. */
export function formatServerTime(value: Date, options: Intl.DateTimeFormatOptions = {
  dateStyle: "medium", timeStyle: "medium",
}): string {
  return `${new Intl.DateTimeFormat("ko-KR", { ...options, timeZone: "UTC" }).format(value)} UTC`;
}
