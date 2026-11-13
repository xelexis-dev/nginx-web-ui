"use client";

import { useId } from "react";
import { Banner, Field, Switch } from "@repo/ui";
import type { RuntimeDnsStatus } from "@repo/contracts";

function unavailableReason(status?: RuntimeDnsStatus): string {
  return status
    ? status.reason || "실행 중 이름 확인 서비스가 준비되지 않았습니다."
    : "실행 중 이름 확인 상태를 확인할 수 없습니다.";
}

export function RuntimeDnsOption({ checked, onChange, status, error, id: controlId, label = "실행 중에 이름 확인" }: {
  checked: boolean;
  id?: string;
  onChange: (enabled: boolean) => void;
  status?: RuntimeDnsStatus;
  error?: string;
  label?: string;
}) {
  const fallbackId = useId();
  const id = controlId ?? fallbackId;
  const unavailable = status?.available !== true;
  return (
    <Field label={label} htmlFor={id} wide error={error} hint={<>
      대상이 꺼져 있어도 설정을 적용할 수 있습니다. 대상이 준비되면 연결을 다시 시도합니다.
      {unavailable ? <><br />{unavailableReason(status)} 이미 선택한 값은 유지되며 끌 수 있습니다.</> : null}
    </>}>
      <Switch checked={checked} onCheckedChange={onChange} label={label} disabled={unavailable && !checked} />
    </Field>
  );
}

export function RuntimeDnsNotice({ status }: { status?: RuntimeDnsStatus; active?: boolean }) {
  if (status?.available === true) return null;
  return <Banner tone="warning" role="status" title={status ? "실행 중 이름 확인을 사용할 수 없습니다" : "실행 중 이름 확인 상태를 확인할 수 없습니다"}>
    {unavailableReason(status)} 저장된 설정은 유지됩니다. DNS 서비스와 포트 설정을 확인하세요.
  </Banner>;
}
