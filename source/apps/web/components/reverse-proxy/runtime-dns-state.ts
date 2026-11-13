/** Pure label shared by server detail pages and client target selectors. */
export function runtimeDnsModeLabel(enabled?: boolean): string {
  return enabled === true ? "실행 중에 이름 확인" : "정적 (설정 적용 시)";
}
