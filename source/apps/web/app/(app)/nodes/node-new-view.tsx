import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { NodeForm } from "./node-form";

// Uses the authenticated Rust API view through the server-side page client.

/**
 * 노드 연결 페이지. 스펙 §8 — 노드 연결은 게이트웨이 admin 전용이라
 * `user:manage` 로 연다(그 액션은 이 저장소에서 admin 만 가진다).
 */
export function NodeNewView({
  defaultGatewayName,
}: {
  defaultGatewayName: string;
}) {
  return (
    <section className={layout.page}>
      <PageHeader title="노드 연결" />
      {/*
        ⚠ **방향을 여기서 못박는다.** 제시할 토큰은 **연결할 그 노드**의
        `/admin/tokens` 에서 나온다 — 이 게이트웨이의 같은 화면에서 만든 토큰은
        「다른 게이트웨이가 우리를 자기 노드로 붙일 때」쓰는 것이라 여기서 아무
        효과가 없다. 거꾸로 적으면 노드를 추가하려던 관리자가 **자기 서버의
        제어권을 넘기는 자격증명**을 만들어 남의 화면에 붙여넣게 된다.
      */}

      <NodeForm defaultGatewayName={defaultGatewayName} />
    </section>
  );
}
