import { NotFoundView } from "@/components/not-found-view";

/**
 * 앱 안에서 `notFound()` 가 던져졌을 때 — 인증서·서버·업스트림·스냅샷처럼 id 로 여는
 * 화면에서 그 항목이 없을 때다. 사이드바가 있는 셸 안에서 뜨므로 단독 배치를 하지 않는다.
 */
export default function AppNotFound() {
  return (
    <NotFoundView
      title="찾는 항목이 없습니다"
      description="삭제됐거나 주소가 잘못됐습니다. 목록에서 다시 확인하세요."
      actions={[
        { href: "/", label: "개요로 가기", primary: true },
        { href: "/certificates", label: "인증서" },
        { href: "/reverse-proxy", label: "리버스 프록시" },
      ]}
    />
  );
}
