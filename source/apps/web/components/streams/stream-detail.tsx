import {
  linkClassName,
  CodePane,
  Facts,
  Icon,
  PageHeader,
  StatusBadge,
  Surface,
} from "@repo/ui";
import Link from "next/link";
import type { RuntimeDnsStatus } from "@repo/contracts";
import { RuntimeDnsNotice } from "@/components/reverse-proxy/runtime-dns";
import { runtimeDnsModeLabel } from "@/components/reverse-proxy/runtime-dns-state";
import { configPreview, type ConfigPreview } from "@/lib/config-preview";
import type { NodeFormAction } from "@/lib/node-action-state";
import { StreamDetailActions } from "./detail-actions";
import layout from "@/components/page-layout.module.css";
import styles from "./stream.module.css";

/** Shared local/node detail. nginx previews are rendered by the target API. */
export interface StreamDetailVM {
  resolveAtRuntime?: boolean;
  preview?: ConfigPreview;
  id: string;
  listen: number;
  protocol: "tcp" | "udp";
  proxyPass: string;
  enabled: boolean;
}

export interface StreamDetailProps {
  runtimeDns?: RuntimeDnsStatus;
  data: StreamDetailVM;
  /** `/stream`(로컬) 또는 `/nodes/<id>/stream`(노드). */
  basePath: string;
  canWrite: boolean;
  /** 로컬은 `toggleStream`, 노드는 `toggleNodeStream.bind(null, nodeId)`. */
  toggleAction: NodeFormAction;
  /** 로컬은 `deleteStream`, 노드는 `deleteNodeStream.bind(null, nodeId)`. */
  deleteAction: NodeFormAction;
}

export function StreamDetail({
  data,
  runtimeDns,
  basePath,
  canWrite,
  toggleAction,
  deleteAction,
}: StreamDetailProps) {
  const {
    config: renderedConfig,
    error: previewError,
    unresolvedPaths,
  } = configPreview(data.preview);

  return (
    <div className={layout.page}>
      <PageHeader
        eyebrow={
          <Link href={basePath} className={linkClassName({ variant: "back" })}>
            <Icon name="arrowLeft" size={14} />
            Stream 목록
          </Link>
        }
        title={`listen ${data.listen}`}
        status={
          <StatusBadge tone={data.enabled ? "success" : "neutral"}>
            {data.enabled ? "활성" : "비활성"}
          </StatusBadge>
        }
        description={data.protocol.toUpperCase()}
        actions={
          canWrite ? (
            <StreamDetailActions
              id={data.id}
              targetName={`listen ${data.listen} · ${data.protocol.toUpperCase()}`}
              enabled={data.enabled}
              toggleAction={toggleAction}
              deleteAction={deleteAction}
              editHref={`${basePath}/${data.id}/edit`}
            />
          ) : undefined
        }
      />

      <RuntimeDnsNotice
        status={runtimeDns}
        active={data.resolveAtRuntime === true}
      />

      <div className={layout.page}>
        <Surface title="설정 요약">
          <Facts
            items={[
              { term: "listen", value: String(data.listen) },
              { term: "protocol", value: data.protocol },
              { term: "proxy_pass", value: data.proxyPass, mono: true },
              {
                term: "이름 확인",
                value: runtimeDnsModeLabel(data.resolveAtRuntime),
              },
              { term: "상태", value: data.enabled ? "활성" : "비활성" },
            ]}
          />
        </Surface>
        <CodePane
          title="생성되는 nginx 설정"
          info={
            <>
              이 항목의 현재 값으로 렌더된 설정입니다 —{" "}
              <strong>설정 적용</strong> 시 nginx <code>stream {"{}"}</code>{" "}
              블록에 기록됩니다.
            </>
          }
          description={
            <>
              {!data.enabled
                ? " 이 stream은 비활성 상태라 적용 시 기록되지 않습니다."
                : ""}
              {unresolvedPaths?.length ? (
                <div role="status" className={styles.previewHint}>
                  이름 확인 없이 만든 미리보기입니다 — 실제 적용에서는 결과가
                  달라질 수 있습니다
                </div>
              ) : null}
            </>
          }
          code={renderedConfig ?? undefined}
          error={
            previewError
              ? `미리보기를 만들 수 없습니다: ${previewError}`
              : undefined
          }
        />
      </div>
    </div>
  );
}
