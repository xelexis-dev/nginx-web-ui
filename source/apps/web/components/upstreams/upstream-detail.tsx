import layout from "@/components/page-layout.module.css";
import { linkClassName, Facts, Icon, PageHeader, Surface } from "@repo/ui";
import Link from "next/link";
import type { RuntimeDnsStatus } from "@repo/contracts";
import { RuntimeDnsNotice } from "@/components/reverse-proxy/runtime-dns";
import { runtimeDnsModeLabel } from "@/components/reverse-proxy/runtime-dns-state";
import type { NodeFormAction } from "@/lib/node-action-state";
import { UpstreamDetailActions } from "./detail-actions";

/**
 * Upstream 상세 화면의 본문. 로컬(`app/(app)/reverse-proxy/upstreams/[id]/page.tsx`)
 * 과 노드(`app/(node)/nodes/[id]/reverse-proxy/upstreams/[upstreamId]/page.tsx`)가
 * 같은 것을 렌더한다. 스타일시트를 왜 복사하지 않고 `(app)` 에서 들여오는지,
 * 그리고 공유 컴포넌트 CSS 규칙·불변식은 `upstream-list.tsx` 헤더에 있다.
 *
 * **이 파일은 서버 컴포넌트로 남는다**(`"use client"` 없음) — 추출 전 로컬
 * 상세가 RSC 였고, 클라이언트 훅이 필요한 것은 삭제 버튼뿐이다. 삭제 액션은
 * 여기서 `UpstreamDetailActions`(클라이언트) 로 **prop 으로 내려간다**: 서버
 * 액션을 클라이언트 컴포넌트에 넘기는 이 계획의 표준 모양이다(스펙 §7).
 */

export interface UpstreamDetailVM {
  resolveAtRuntime?: boolean;
  id: string;
  name: string;
  /** 이미 문자열 배열로 정규화돼 있다 — `UpstreamRowVM.servers` 와 같은 규약. */
  servers: string[];
}

export interface UpstreamDetailProps {
  runtimeDns?: RuntimeDnsStatus;
  data: UpstreamDetailVM;
  /** `/reverse-proxy/upstreams`(로컬) 또는 `/nodes/<id>/reverse-proxy/upstreams`(노드). */
  basePath: string;
  canWrite: boolean;
  /** 로컬은 `deleteUpstream`, 노드는 `deleteNodeUpstream.bind(null, nodeId)`. */
  deleteAction: NodeFormAction;
}

export function UpstreamDetail({
  data,
  runtimeDns,
  basePath,
  canWrite,
  deleteAction,
}: UpstreamDetailProps) {
  return (
    <div className={layout.page}>
      <PageHeader
        eyebrow={
          <Link href={basePath} className={linkClassName({ variant: "back" })}>
            <Icon name="back" size={14} />
            Upstreams
          </Link>
        }
        title={data.name}
        description={`서버 ${data.servers.length}개`}
        actions={
          canWrite ? (
            <UpstreamDetailActions
              id={data.id}
              targetName={data.name}
              action={deleteAction}
              editHref={`${basePath}/${data.id}/edit`}
            />
          ) : undefined
        }
      />

      <RuntimeDnsNotice
        status={runtimeDns}
        active={data.resolveAtRuntime === true}
      />

      <Surface title="설정 요약">
        <Facts
          items={[
            { term: "이름", value: data.name },
            {
              term: "이름 확인",
              value: runtimeDnsModeLabel(data.resolveAtRuntime),
            },
            {
              term: "서버",
              value: data.servers.length > 0 ? data.servers.join(", ") : "—",
              mono: data.servers.length > 0,
            },
          ]}
        />
      </Surface>
    </div>
  );
}
