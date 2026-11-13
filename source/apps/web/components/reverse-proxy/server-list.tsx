import type { ReactNode } from "react";
import layout from "@/components/page-layout.module.css";
import Link from "next/link";
import {
  Banner,
  Icon,
  Facts,
  PageHeader,
  buttonClassName,
} from "@repo/ui";
import type { ConfigDiff } from "@/lib/config-diff";
import type { NodeFormAction } from "@/lib/node-action-state";
import { ServerTable } from "./server-table";
import type { ServerOpsRowVM } from "./server-status";

/**
 * Reverse Proxy 목록 화면의 본문(Operations) — 머리말 + 서버 표. 로컬 화면
 * (`app/(app)/reverse-proxy/page.tsx`)과 노드 화면(`app/(node)/nodes/[id]/reverse-proxy/page.tsx`)이
 * 같은 것을 렌더한다. 다른 것은 데이터 출처(로컬 뷰 vs 노드 읽기)와 넘기는 명령뿐이다.
 *
 * **「설정 적용」은 여기 없다** — 셸 머리줄의 적용 버튼(미적용 수와 diff, nginx -t 결과를 보여 주는
 * 기존 ApplyBar)이 모든 화면에서 같은 자리에 있다(`components/operations/operations-shell.tsx`).
 *
 * **불변식: 공유 컴포넌트는 라우트 그룹의 CSS 모듈은 가져다 써도 TS/TSX 는 절대 가져오지 않는다**
 * (`components/upstreams/upstream-list.tsx` 헤더).
 *
 * **`basePath` 하나로 화면 안의 모든 링크가 결정된다** — 행 링크·Upstreams·「+ 새 서버」·접속 지표가
 * 전부 여기서 갈린다. 노드 경로가 `/nodes/<id>/reverse-proxy`(로컬 `/reverse-proxy` 와 같은 꼬리)여야
 * 하는 이유가 이것이다.
 */

export interface ServerListVM {
  /** 행마다 TLS·상태·미적용이 붙은 줄(`toOpsRow`). 인증서는 같은 출처의 보조 읽기에서 온다. */
  items: ServerOpsRowVM[];
  /** 인증서 보조 읽기가 실패했다 — TLS 칸이 「확인 불가」인 이유를 한 줄로 알린다. */
  certificatesUnknown: boolean;
  /** `nginx:reload`. 적용 버튼은 셸에 있지만 판정은 이 화면의 읽기와 같은 출처다. */
  canApply: boolean;
  /**
   * 미적용 변경 수. **노드 화면에서는 노드의 `/api/v1/pending` 값이어야 한다** —
   * 게이트웨이 자기 것을 실으면 「적용할 게 없는데 수가 떠 있다」가 된다.
   */
  pendingCount: number;
  pendingHasBaseline: boolean;
  /** 행의 「미적용」 표시가 읽는 diff. `pendingCount` 와 같은 응답에서 온 것이라 둘이 어긋나지 않는다. */
  pendingDiff: ConfigDiff;
}

/**
 * 쓰기 UI 한 덩어리. **`canWrite: boolean` 하나로 대신하지 않는 것이 요점이다** — 그러면 「그릴까
 * 말까」와 「어디로 보낼까」가 갈라져서, 아직 없는 경로에도 버튼이 그려지는 조합을 아무도 못 잡는다.
 * 그래서 목적지와 명령을 전부 값으로 들고 있다.
 */
export interface ServerListWrite {
  /** 「+ 새 서버」가 갈 곳. */
  newHref: string;
  /**
   * 「템플릿으로 생성」이 갈 곳. **노드에는 템플릿 화면이 없다**(로컬 전용
   * `(app)/reverse-proxy/templates`) — 그때는 `null` 이고 버튼이 사라진다.
   */
  templatesHref: string | null;
  /** 행 스위치가 보낼 명령 — 로컬 `reverse-proxy.toggleServer`, 노드 `node.toggleNodeServer`. */
  toggleAction: NodeFormAction;
}

export interface ServerListProps {
  data: ServerListVM;
  navigation?: ReactNode;
  /** `/reverse-proxy`(로컬) 또는 `/nodes/<id>/reverse-proxy`(노드). */
  basePath: string;
  /**
   * 쓰기 UI. **등급이 모자라거나(viewer) 아직 쓰기 경로가 없는 화면이면 `null`** 이고 그때는
   * 「+ 새 서버」·「템플릿으로 생성」·행 스위치·「수정」을 그리지 않는다.
   */
  write: ServerListWrite | null;
}

export function ServerList({
  data,
  basePath,
  write,
  navigation,
}: ServerListProps) {
  const { items, certificatesUnknown, pendingCount } = data;
  const active = items.filter((s) => s.enabled).length;

  return (
    <div className={layout.page}>
      <PageHeader
        title="리버스 프록시"
        info="상태는 저장된 설정과 인증서 기준입니다(서버 응답 확인이 아님). 트래픽은 접속 지표에서 봅니다."
        actions={
          <>
            {!navigation ? (
              <Link
                href={`${basePath}/upstreams`}
                className={buttonClassName({ variant: "secondary" })}
              >
                Upstreams 관리
              </Link>
            ) : null}
            {write?.templatesHref ? (
              <Link
                href={write.templatesHref}
                className={buttonClassName({ variant: "secondary" })}
              >
                템플릿으로 생성
              </Link>
            ) : null}
            {write ? (
              <Link href={write.newHref} className={buttonClassName()}>
                <Icon name="plus" />새 서버
              </Link>
            ) : null}
          </>
        }
      />
      {navigation}
      <Facts
        layout="strip"
        items={[
          { term: "서버", value: `${items.length}개` },
          { term: "활성", value: active },
          {
            term: "미적용",
            value: data.pendingHasBaseline
              ? pendingCount
              : "적용 상태 확인 필요",
          },
        ]}
      />
      {certificatesUnknown ? (
        <Banner
          tone="warning"
          title="인증서 상태를 읽지 못했습니다"
          role="status"
        >
          TLS 칸과 인증서에 따른 상태를 확인할 수 없어 「확인 불가」로
          표시합니다.
        </Banner>
      ) : null}
      <ServerTable
        servers={items}
        basePath={basePath}
        write={write ? { toggleAction: write.toggleAction } : null}

      />
    </div>
  );
}
