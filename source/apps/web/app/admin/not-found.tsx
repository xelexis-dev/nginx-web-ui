import { NotFoundView } from "@/components/not-found-view";

/**
 * 관리자 패널 안에서 `notFound()` 가 던져졌을 때 — 메일 수신처·webhook 수신처처럼
 * id 로 여는 화면에서 그 항목이 없을 때다. 관리자 셸(전용 사이드바) 안에서 뜬다.
 */
export default function AdminNotFound() {
  return (
    <NotFoundView
      title="찾는 항목이 없습니다"
      description="삭제됐거나 주소가 잘못됐습니다. 목록에서 다시 확인하세요."
      actions={[
        { href: "/admin/users", label: "관리자 홈으로", primary: true },
        { href: "/admin/notifications/mail", label: "알림 설정" },
      ]}
    />
  );
}
