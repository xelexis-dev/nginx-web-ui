export function adminPasskeyRemovedMessage(isSelf: boolean): string {
  return isSelf
    ? "패스키를 삭제했습니다. 다른 기기의 로그인은 종료되고 이 기기의 로그인은 유지됩니다."
    : "패스키를 삭제하고 사용자의 로그인을 종료했습니다.";
}
