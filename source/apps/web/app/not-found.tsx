import { NotFoundView } from "@/components/not-found-view";

/**
 * 알 수 없는 주소. 앱 셸 밖에서 단독으로 뜬다(루트 레이아웃만 적용).
 *
 * 로그인하지 않은 채로 여기 닿는 일은 거의 없다 — 미들웨어가 `/login` 으로 먼저
 * 보내기 때문이다. 즉 이 화면을 보는 사람은 대개 로그인한 운영자다.
 */
export default function NotFound() {
  return (
    <NotFoundView
      standalone
      title="이 주소에는 아무것도 없습니다"
      description="주소를 잘못 입력했거나, 링크가 지금은 없는 화면을 가리키고 있습니다."
      actions={[{ href: "/", label: "개요로 가기", primary: true }]}
    />
  );
}
