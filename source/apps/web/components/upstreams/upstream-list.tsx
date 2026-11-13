import type { ReactNode } from "react";
import layout from "@/components/page-layout.module.css";
import {
  EmptyState,
  Facts,
  Icon,
  PageHeader,
  Surface,
  Table,
  Td,
  Th,
  Tr,
  buttonClassName,
} from "@repo/ui";
import Link from "next/link";

/**
 * Upstream 목록 화면의 본문. 로컬 화면(`app/(app)/reverse-proxy/upstreams/page.tsx`)
 * 과 노드 화면(`app/(node)/nodes/[id]/reverse-proxy/upstreams/page.tsx`)이 같은
 * 것을 렌더한다 — 데이터 출처만 다르다(로컬 DB vs `loadNodePage`).
 * `components/` 에 있는 이유는 `(app)` 과 `(node)` 두 라우트 그룹이 함께 쓰기
 * 때문이다(`components/audit/audit-list.tsx` 와 같은 이유).
 *
 * ---
 *
 * **공유 컴포넌트의 CSS 규칙(이 디렉터리 전체에 적용):** 클래스의 정의는 **한
 * 벌**이다. **옮길 수 있으면 옮기고**(`components/audit/audit.module.css`,
 * `components/history/history.module.css` 가 그렇게 만들어졌다 — 복사가 아니라
 * 이동이고, `(app)/audit/` 에는 모듈이 아예 안 남았다), `composes` 나 로컬 전용
 * 형제 때문에 **못 옮기면 라우트 그룹의 모듈을 가져다 쓴다.** 어느 쪽인지와
 * 왜인지를 파일 헤더에 적는다.
 *
 * **여기는 「못 옮기는」 쪽이다** — 그래서 `(app)/reverse-proxy` 의 시트를 그대로
 * 들여온다. 이유 둘 다 구조적이다(비싸서가 아니다):
 *
 * 1. 이 화면들이 쓰는 클래스를 **로컬로 남는 리버스 프록시 서버 화면들**
 *    (`server-table.tsx`, `server-form.tsx`, `[id]/page.tsx`, `form-sections/*`)이
 *    같이 쓴다. 감사·히스토리 때는 그 클래스들이 그 화면 전용이라 옮길 수 있었다.
 * 2. `reverse-proxy.module.css` 가 `composes` 를 쓴다(`.buttonDanger composes:
 *    buttonSecondary` — 둘 다 upstream 화면이 쓴다). **`composes` 는 한 모듈
 *    안에서만 풀리므로** 쪼개면 기반 클래스를 양쪽에 복제하거나 로컬 서버 폼을
 *    깨뜨린다.
 *
 * 덤으로 로컬 화면은 추출 전과 **글자 그대로 같은 클래스**를 받는다.
 *
 * **불변식: 공유 컴포넌트는 라우트 그룹의 CSS 모듈은 가져다 써도 그 그룹의
 * TS/TSX 는 절대 가져오지 않는다.** CSS 모듈은 import 가 없는 잎이라 순환이
 * 생기지 않지만, TS 를 가져오는 순간 `(app)` ↔ `components` 진짜 순환이 된다.
 *
 * ---
 *
 * **`basePath` 하나로 화면 안의 모든 upstream 링크가 결정된다.** 노드 경로가
 * `/nodes/<id>/reverse-proxy/upstreams`(로컬 `/reverse-proxy/upstreams` 와 같은
 * 꼬리) 여야 하는 이유가 이것이다 — 꼬리가 다르면 치환이 한 번으로 안 끝난다.
 */

export interface UpstreamRowVM {
  id: string;
  name: string;
  /**
   * 이미 문자열 배열로 정규화돼 있다. 로컬은 jsonb 컬럼에서, 노드는 JSON 응답
   * 에서 각각 옮긴다 — 이 컴포넌트가 `Array.isArray` 를 다시 하지 않도록.
   */
  servers: string[];
}

export interface UpstreamListVM {
  items: UpstreamRowVM[];
  /**
   * 「+ Upstream 추가」를 보여줄지. 로컬은 `can(session.user.role, ...)`,
   * 노드는 노드 토큰 등급과 min 을 취한 유효 권한이다(스펙 §8).
   */
  canWrite: boolean;
}

export interface UpstreamListProps {
  data: UpstreamListVM;
  navigation?: ReactNode;
  /** `/reverse-proxy/upstreams`(로컬) 또는 `/nodes/<id>/reverse-proxy/upstreams`(노드). */
  basePath: string;
}

export function UpstreamList({
  data,
  basePath,
  navigation,
}: UpstreamListProps) {
  const { items, canWrite } = data;

  return (
    <div className={layout.page}>
      <PageHeader
        title="Upstreams"
        info="Reverse Proxy 서버가 참조하는 upstream 그룹을 관리합니다."
        actions={
          canWrite ? (
            <Link href={`${basePath}/new`} className={buttonClassName()}>
              <Icon name="plus" />
              Upstream 추가
            </Link>
          ) : null
        }
      />

      {navigation}
      <Facts
        layout="strip"
        items={[
          { term: "Upstreams", value: items.length },
          {
            term: "서버",
            value: items.reduce(
              (total, item) => total + item.servers.length,
              0,
            ),
          },
        ]}
      />
      <Surface flush>
        {items.length === 0 ? (
          <EmptyState title="등록된 upstream이 없습니다" />
        ) : (
          <Table label="Upstreams">
            <thead>
              <tr>
                <Th>이름</Th>
                <Th>서버</Th>
              </tr>
            </thead>
            <tbody>
              {items.map((u) => (
                <Tr key={u.id}>
                  <Td cell="primary" icon={<Icon name="server" size={16} />}>
                    <Link
                      href={`${basePath}/${u.id}`}
                    >
                      {u.name}
                    </Link>
                  </Td>
                  <Td mono wrap label="서버">
                    {u.servers.join(", ")}
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Surface>
    </div>
  );
}
