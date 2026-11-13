import { redirect } from "next/navigation";

/**
 * SMTP 서버 설정은 사이트 설정(`/admin/settings/smtp`)으로 옮겼다 — `/admin/notifications/mail`
 * 은 이제 "무엇을 누구에게"를 정하는 알림 채널 화면이다. 북마크·문서에 남은 옛 주소를
 * 깨뜨리지 않으려고 리다이렉트만 남긴다.
 */
export default function SmtpRedirectPage() {
  redirect("/admin/settings/smtp");
}
