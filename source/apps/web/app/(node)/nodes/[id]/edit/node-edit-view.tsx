import layout from "@/components/page-layout.module.css";
import { Icon, linkClassName, PageHeader, Card, FormStack } from "@repo/ui";
import Link from "next/link";
import type { NodeView } from "@/lib/node-page";
import { NodeEditForm } from "./node-edit-form";
import { UnpairButton } from "./unpair-button";
import styles from "../../../../(app)/nodes/nodes.module.css";

// Uses the authenticated Rust API view through the server-side page client.

/**
 * 노드 수정 화면: 이름 / 주소 / 자체서명 허용만 바꾼다 + 연결 해제.
 *
 * 스펙 §8 — 노드 연결/해제와 마찬가지로 이 화면도 게이트웨이 admin 전용이다
 * (`user:manage`). 이 화면은 원격 호출을 하지 않는다(저장은 로컬 `nodes` 행만
 * 바꾼다) — 그래서 `resolveNode`(노드 토큰 등급까지 min 을 취하는 판정)가 아니라
 * `requireRole` 로 게이트웨이 쪽 등급만 본다. 노드가 죽어 있어도, 토큰 등급이
 * 낮아도 이 화면엔 닿을 수 있어야 한다 — 그럴 때일수록 주소를 고치거나 연결을
 * 끊어야 하기 때문이다(요약 화면의 「수정」 링크 gate 와 같은 근거 — `[id]/page.tsx`
 * 참고).
 */
export function NodeEditView({ node }: { node: NodeView }) {
  const id = node.id;

  return (
    <section className={layout.page}>
      <PageHeader
        title={<>{node.name} 수정</>}
        eyebrow={
          <>
            {" "}
            <Link
              href={`/nodes/${id}`}
              className={linkClassName({ variant: "back" })}
            >
              <Icon name="arrowLeft" size={14} />
              상세로
            </Link>{" "}
          </>
        }
      />

      <FormStack>
        <NodeEditForm
          id={node.id}
          defaults={{
            name: node.name,
            baseUrl: node.baseUrl,
            allowInsecureTls: node.allowInsecureTls,
          }}
          tokenRole={node.tokenRole}
          registeredGatewayName={node.gatewayName}
          unpair={
            <Card id="node-unpair" title="연결 해제">
              <p className={styles.hint}>
                노드와의 연결을 끊습니다. 노드에 등록된 nginx 설정은 그대로
                남고, 이 게이트웨이의 노드 목록에서만 사라집니다.
              </p>
              <div className={styles.detailActionsRow}>
                <UnpairButton id={node.id} name={node.name} />
              </div>
            </Card>
          }
        />
      </FormStack>
    </section>
  );
}
