"use client";
import { UNAPPLIED_CONFIG_INFO } from "@/components/reverse-proxy/save-info";

import {
  buttonClassName,
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
import { Button, Select } from "@repo/ui";
// 이 폼이 `NodeErrorCard` 를 들여오는 것은 **의도한 것이다** — 로컬·노드가 같은
// 컴포넌트를 쓰는 것이 이 계획의 방식이라, 노드 액션이 넘어올 자리를 로컬 화면이
// 그리는 폼도 그대로 갖고 있어야 한다(`components/upstreams/upstream-form.tsx`
// 와 같은 판단). 그 카드는 클라이언트에서 쓰이도록 만든 것이고 `node:tls` 사슬을
// 끌고 오지 않는다(그 파일 헤더).
import { NodeErrorCard } from "@/components/node-error";
import type { NodeActionState, NodeFormAction } from "@/lib/node-action-state";
import { useApiAction } from "@/lib/use-api-action";
import { targetLabel, type OperationsTarget } from "@repo/presentation/operations-target";
import layout from "@/components/page-layout.module.css";

const INITIAL: NodeActionState = { ok: false };
const ERROR_FIELDS: Record<string, string> = {"listen": "stream-listen", "protocol": "stream-protocol", "proxyPass": "stream-proxy-pass", "resolveAtRuntime": "stream-runtime-dns"};

interface StreamFormProps {
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
    listen: number;
    protocol: "tcp" | "udp";
    proxyPass: string;
    version?: string; resolveAtRuntime?: boolean;
  };
  /** Where "취소" links back to (the list for /new, the detail page for /edit). */
  cancelHref: string;
  target?: OperationsTarget;
  submitLabel?: string;
  /**
   * 이 인스턴스가 이미 쓰고 있어 **stream 이 잡으면 안 되는** 포트. 로컬 화면은
   * `reservedPorts()` 로 실제 값을 넘기고, **노드 화면은 넘기지 않는다** — 그 값은
   * 노드의 env 에서 나오는 것이라 게이트웨이가 알 수 없다. 게이트웨이가 자기 값을
   * 대신 보여주면 남의 인스턴스에 대해 **틀린 숫자**를 안내하게 된다. 노드 쪽은
   * 숫자 없는 문장만 보이고, 실제 판정은 저장할 때 노드가 하고 그 오류가 위
   * `state.fieldErrors.listen` 으로 그대로 돌아온다.
   */
  reservedListenPorts?: readonly number[];
  /** 대상 인스턴스의 UDP 전용 예약 목록. undefined는 대상의 목록을 모르는 경우다. */
  reservedUdpListenPorts?: readonly number[];
}

/**
 * Create/edit page form for one L4 stream (listen, protocol tcp|udp,
 * proxy_pass host:port). On success `action` redirects away (to the detail
 * page) — this component never sees an `ok: true` result. Only
 * field-validation errors round-trip back here via `useActionState`.
 */
export function StreamForm({
  action,
  runtimeDns,
  nodeVersionFamily,
  defaultValues,
  cancelHref,
  target,
  submitLabel = "저장",
  reservedListenPorts,
  reservedUdpListenPorts,
}: StreamFormProps) {
  // A props refresh must not advance the comparison version of the retained draft.
  const [version] = useState(defaultValues?.version);
  const [renderedFamily] = useState(nodeVersionFamily);
  const [state, formAction, pending] = useActionState(
    useApiAction<NodeActionState>(action),
    INITIAL,
  );
  const [listen, setListen] = useState(String(defaultValues?.listen ?? ""));
  const [proxyPass, setProxyPass] = useState(defaultValues?.proxyPass ?? "");
  const [resolveAtRuntime, setResolveAtRuntime] = useState(defaultValues?.resolveAtRuntime === true);

  const [protocol, setProtocol] = useState<"tcp" | "udp">(defaultValues?.protocol ?? "tcp");
  const portsText = (ports: readonly number[]) => [...new Set(ports)].sort((a, b) => a - b).join(", ");
  const reservedHint = protocol === "udp"
    ? reservedUdpListenPorts === undefined
      ? "UDP stream은 대상 서버의 이름 확인 서비스 포트와 겹치면 쓸 수 없습니다. 정확한 충돌은 저장할 때 대상 서버가 확인합니다."
      : reservedUdpListenPorts.length > 0
        ? `UDP stream은 이 인스턴스가 이미 듣는 UDP 포트(${portsText(reservedUdpListenPorts)})를 쓸 수 없습니다.`
        : "이 인스턴스에 예약된 UDP 프로세스 포트가 없습니다. TCP 전용 예약 포트는 UDP에 적용되지 않습니다."
    : reservedListenPorts && reservedListenPorts.length > 0
      ? `TCP stream은 이 인스턴스가 이미 듣는 포트(${portsText(reservedListenPorts)})를 쓸 수 없습니다.`
      : "TCP stream은 대상 서버의 관리 화면·웹 앱·데이터베이스·기존 프록시 포트를 쓸 수 없습니다. 정확한 충돌은 저장할 때 대상 서버가 확인합니다.";

  return (
    <form action={formAction} className={layout.page}>
      {renderedFamily && <input type="hidden" name="nodeVersionFamily" value={renderedFamily} />}
      {/* 노드 액션이 낸 「닿지 못함」·「버전 불일치」 등은 한 줄 빨간 글씨가
            아니라 §9 카드다. 로컬 액션은 `nodeError` 를 채우는 일이 없어 아래
            `state.error` 줄로 그대로 떨어진다(동작 무변화). */}
      {state.nodeError ? (
        <NodeErrorCard {...state.nodeError} />
      ) : state.error || Object.keys(state.fieldErrors ?? {}).length > 0 ? (
        <Banner tone="error" role="alert" title={state.error}>
          {Object.entries(state.fieldErrors ?? {}).map(([name, message]) => (
            <div key={name}><a className={linkClassName()} href={`#${ERROR_FIELDS[name] ?? "stream-settings-heading"}`}>{message}</a></div>
          ))}
        </Banner>
      ) : null}
      <FormStack>
        <Banner tone="info" title={UNAPPLIED_CONFIG_INFO} />
        <Card id="stream-settings" title="Stream" errorCount={Object.keys(state.fieldErrors ?? {}).length}>
          <div className={formFieldsClassName()} data-rollout-form-grid>
            {version !== undefined ? <input type="hidden" name="version" value={version} /> : null}
            <input type="hidden" name="resolveAtRuntime" value={String(resolveAtRuntime)} />
            {defaultValues ? (
              <input type="hidden" name="id" value={defaultValues.id} />
            ) : null}

            {/* **기본값을 두지 않는다.** 예전 기본값 5432 는 이 컨테이너 자신의 DB
                포트였다 — 폼을 그대로 저장하면 적용은 초록 성공을 보고하고, 다음
                재시작에서 nginx 가 5432 를 먼저 잡아 postgres 가 뜨지 못해 인스턴스
                전체가 죽었다. 채워진 기본값은 「이 값이 맞다」는 신호라, 빈 칸이
                맞다. (힌트 id 는 Field 가 그대로 stream-listen-hint 로 단다.) */}
            <Field
              label="listen (포트)"
              htmlFor="stream-listen"
              hint={reservedHint}
              error={state.fieldErrors?.listen}
            >
              <TextInput
                name="listen"
                type="number"
                min={1}
                max={65535}
                value={listen}
                onChange={(event) => setListen(event.target.value)}
                placeholder="예: 15432"
                required
              />
            </Field>

            <Field
              label="protocol"
              htmlFor="stream-protocol"
              error={state.fieldErrors?.protocol}
            >
              <Select
                name="protocol"
                value={protocol}
                onChange={(value) => setProtocol(value === "udp" ? "udp" : "tcp")}
                options={[
                  { value: "tcp", label: "tcp" },
                  { value: "udp", label: "udp" },
                ]}
                aria-label="protocol"
              />
            </Field>

            {/* placeholder 에 루프백을 쓰지 않는다. `127.0.0.1:5432` 는 **이 컨테이너
                자신의 DB** 를 가리켜, listen 기본값 5432 와 짝을 이루어 「의도된 예시」로
                읽혔다. 게다가 재시작 뒤에는 nginx 가 5432 를 쥐고 있어 그 대상이 자기
                리스너를 가리킨다. 다른 호스트의 서비스가 L4 프록시의 정상적인 대상이다. */}
            <Field
              label="proxy_pass"
              htmlFor="stream-proxy-pass"
              error={state.fieldErrors?.proxyPass}
            >
              <TextInput
                name="proxyPass"
                placeholder="10.0.0.5:5432"
                value={proxyPass}
                onChange={(event) => setProxyPass(event.target.value)}
                required
              />
            </Field>

            <RuntimeDnsOption id="stream-runtime-dns" checked={resolveAtRuntime} onChange={setResolveAtRuntime} status={runtimeDns} error={state.fieldErrors?.resolveAtRuntime} />

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
