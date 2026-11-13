"use client";
import { UNAPPLIED_CONFIG_INFO } from "@/components/reverse-proxy/save-info";

import {
  buttonClassName,
  TextArea,
  TextInput,
  Field,
  Banner,
  Card,
  formFieldsClassName,
  linkClassName,
  FormStack,
  FormBar,
} from "@repo/ui";

import { useActionState, useState } from "react";
import type { RuntimeDnsStatus } from "@repo/contracts";
import { RuntimeDnsOption } from "@/components/reverse-proxy/runtime-dns";
import Link from "next/link";
import { Button } from "@repo/ui";
// 이 폼이 `NodeErrorCard` 를 들여오는 것은 **의도한 것이다** — 로컬·노드가 같은
// 컴포넌트를 쓰는 것이 이 계획의 방식이라, 노드 액션이 넘어올 자리를 로컬 화면이
// 그리는 폼도 그대로 갖고 있어야 한다. 그 카드는 클라이언트에서 쓰이도록 만든
// 것이고(그 파일 헤더) `node:tls` 사슬을 끌고 오지 않는다(`yarn build` 로 확인).
// 로컬 청크가 조금 무거워지는 것은 그 대가다 — 회귀가 아니다.
import { NodeErrorCard } from "@/components/node-error";
import type { NodeActionState, NodeFormAction } from "@/lib/node-action-state";
import { useApiAction } from "@/lib/use-api-action";
// 라우트 그룹의 시트를 그대로 들여온다 — 왜 복사하지 않는지(그리고 공유 컴포넌트
// CSS 규칙·불변식)는 `upstream-list.tsx` 헤더 참고.
import {
  targetLabel,
  type OperationsTarget,
} from "@repo/presentation/operations-target";
import layout from "@/components/page-layout.module.css";

const INITIAL: NodeActionState = { ok: false };
const ERROR_FIELDS: Record<string, string> = {"name": "upstream-name", "servers": "upstream-servers", "resolveAtRuntime": "upstream-runtime-dns"};

interface UpstreamFormProps {
  /**
   * 로컬 액션도 노드 액션도 그대로 들어간다 — `NodeFormAction` 이 기존
   * `ActionState` 액션의 상위 계약이다(그 타입의 문서). **`ActionState` 로
   * 되돌리지 마라**: 추가 필드가 전부 optional 이라 컴파일은 그대로 통과하는데
   * 아래 `state.nodeError` 가 타입에서 사라져 §9 카드가 영영 안 그려진다.
   */
  action: NodeFormAction;
  runtimeDns?: RuntimeDnsStatus;
  nodeVersionFamily?: "1.19" | "2.0";
  defaultValues?: {
    id: string;
    name: string;
    servers: string[];
    version?: string;
    resolveAtRuntime?: boolean;
  };
  /** Where "취소" links back to (the list for /new, the detail page for /edit). */
  cancelHref: string;
  submitLabel?: string;
  target?: OperationsTarget;
}

/**
 * Create/edit page form for one upstream (name + newline-separated server
 * list). On success `action` redirects away (to the detail page) — only
 * field-validation errors round-trip back here via `useActionState`.
 */
export function UpstreamForm({
  action,
  runtimeDns,
  nodeVersionFamily,
  defaultValues,
  cancelHref,
  submitLabel = "저장",
  target,
}: UpstreamFormProps) {
  // A props refresh must not advance the comparison version of the retained draft.
  const [version] = useState(defaultValues?.version);
  const [renderedFamily] = useState(nodeVersionFamily);
  const [state, formAction, pending] = useActionState(
    useApiAction<NodeActionState>(action),
    INITIAL,
  );
  const [name, setName] = useState(defaultValues?.name ?? "");
  const [servers, setServers] = useState(
    defaultValues?.servers.join("\n") ?? "",
  );
  const [resolveAtRuntime, setResolveAtRuntime] = useState(
    defaultValues?.resolveAtRuntime === true,
  );

  return (
    <form action={formAction} className={layout.page}>
      {renderedFamily && <input type="hidden" name="nodeVersionFamily" value={renderedFamily} />}
      {version !== undefined ? (
        <input type="hidden" name="version" value={version} />
      ) : null}
      <input
        type="hidden"
        name="resolveAtRuntime"
        value={String(resolveAtRuntime)}
      />
      {defaultValues ? (
        <input type="hidden" name="id" value={defaultValues.id} />
      ) : null}

      {/* 노드 액션이 낸 「닿지 못함」·「버전 불일치」 등은 한 줄 빨간 글씨가
          아니라 §9 카드다 — 어느 쪽을 올려야 하는지, 다시 연결하려면 어디로
          가야 하는지를 그 카드만 말해준다. 로컬 액션은 `nodeError` 를 채우는
          일이 없어 아래 `state.error` 줄로 그대로 떨어진다(동작 무변화). */}
      {state.nodeError ? (
        <NodeErrorCard {...state.nodeError} />
      ) : state.error || Object.keys(state.fieldErrors ?? {}).length > 0 ? (
        <Banner tone="error" role="alert" title={state.error}>
          {Object.entries(state.fieldErrors ?? {}).map(([name, message]) => (
            <div key={name}><a className={linkClassName()} href={`#${ERROR_FIELDS[name] ?? "upstream-form-fields-heading"}`}>{message}</a></div>
          ))}
        </Banner>
      ) : null}
      <FormStack>
        <Banner tone="info" title={UNAPPLIED_CONFIG_INFO} />
        <Card
          id="upstream-form-fields"
          title="Upstream"
          errorCount={Object.keys(state.fieldErrors ?? {}).length}
        >
          <div className={formFieldsClassName()} data-rollout-form-grid>
            <Field
              label="이름"
              required
              htmlFor="upstream-name"
              error={state.fieldErrors?.name}
            >
              <TextInput
                name="name"
                value={name}
                onChange={(event) => setName(event.target.value)}
                required
              />
            </Field>

            <Field
              label="서버 목록 (한 줄에 하나씩)"
              htmlFor="upstream-servers"
              error={state.fieldErrors?.servers}
              wide
            >
              <TextArea
                mono
                name="servers"
                rows={4}
                placeholder={"127.0.0.1:3000\n127.0.0.1:3001"}
                value={servers}
                onChange={(event) => setServers(event.target.value)}
              />
            </Field>

            <RuntimeDnsOption
              id="upstream-runtime-dns"
              checked={resolveAtRuntime}
              onChange={setResolveAtRuntime}
              status={runtimeDns}
              error={state.fieldErrors?.resolveAtRuntime}
            />

          </div>
        </Card>
      </FormStack>
      <FormBar note={target?.kind === "node" ? <>저장 대상: {targetLabel(target)}</> : undefined}>
        <Link href={cancelHref} className={buttonClassName({ variant: "secondary" })}>
          취소
        </Link>
        <Button type="submit" disabled={pending}>
          {pending ? "저장 중..." : submitLabel}
        </Button>
      </FormBar>
    </form>
  );
}
