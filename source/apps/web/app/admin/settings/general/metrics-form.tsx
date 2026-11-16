"use client";
import { Card, FormStack } from "@repo/ui";
import adminStyles from "@/app/admin/admin-pages.module.css";
import resourceStyles from "@/components/resource-layout.module.css";
import { FormErrors } from "@/components/form-errors";
import { useActionState, useState } from "react";
import {
  Banner,
  TextArea,
  TextInput,
  Button,
  Field,
  formFieldsClassName,
} from "@repo/ui";
import { useApiAction } from "@/lib/use-api-action";
import type { GeneralSettingsState } from "./form-types";

const INITIAL: GeneralSettingsState = { ok: false };

export function MetricsForm({
  defaultRetentionDays,
  defaultExcludedHosts,
}: {
  defaultRetentionDays: string;
  defaultExcludedHosts: string;
}) {
  const [state, action, pending] = useActionState(
    useApiAction<GeneralSettingsState>({
      id: "admin.settings.updateMetrics",
      args: [],
    }),
    INITIAL,
  );
  const [days, setDays] = useState(defaultRetentionDays);
  const [hosts, setHosts] = useState(defaultExcludedHosts);

  return (
    <form className={resourceStyles.form} action={action}>
      <FormErrors errors={state.fieldErrors} />
      <FormStack>
        <Card
          id="admin-metrics"
          title="접속 지표"
          info="접속 지표를 얼마나 오래 보관할지, 어떤 도메인을 집계에서 뺄지 정합니다."
          errorCount={
            Object.values(state.fieldErrors ?? {}).filter(Boolean).length
          }
        >
          <div className={adminStyles.body}>
            <div className={formFieldsClassName()}>
              <Field
                label="보존 기간 (일)"
                htmlFor="metrics-retention-days"
                hint={
                  <>
                    무제한으로 두면 1분 간격 기준 연 약 50 MB, 도메인별 집계까지
                    합쳐 연 약 300 MB 늘어납니다. 보존 기간을 줄여도{" "}
                    <strong>
                      이미 확보된 디스크 공간은 즉시 반환되지 않고
                    </strong>{" "}
                    새 데이터에 재사용됩니다.
                  </>
                }
                error={state.fieldErrors?.metricsRetentionDays}
              >
                <TextInput
                  name="metricsRetentionDays"
                  inputMode="numeric"
                  value={days}
                  onChange={(e) => setDays(e.target.value)}
                  placeholder="비우면 무제한"
                />
              </Field>

              <Field
                label="집계 제외 도메인"
                htmlFor="metrics-excluded-hosts"
                hint={
                  <>
                    <p>
                      한 줄에 하나씩. 이 관리 화면이 도메인 뒤에 있다면 그
                      도메인을 넣으세요.
                    </p>
                    <p>
                      <strong>적용은 지금부터입니다.</strong> 제외는 기록을 쌓는
                      시점에 걸리므로,{" "}
                      <strong>이미 쌓인 과거 구간은 그대로 남습니다</strong> —
                      어제 그래프가 안 바뀌었다고 설정이 안 먹은 것은 아닙니다.
                    </p>
                    <p>
                      기록은 몇 분씩 모아 처리하므로,{" "}
                      <strong>저장 직전 몇 분치가 함께 빠질 수 있습니다</strong>{" "}
                      — 그래프가 저장 시각보다 조금 앞에서 끊겨 보이면 그
                      때문이며, 정상입니다.
                    </p>
                  </>
                }
                info={
                  <>
                    <p>
                      관리 UI 요청은 nginx 를 두 번 지나 요청 수가 두 배로
                      잡힙니다.
                    </p>
                    <p>
                      과거까지 정리하려면 보존 기간을 줄여 그 구간이 지나가길
                      기다려야 합니다.
                    </p>
                    <p>
                      경계는{" "}
                      <strong>
                        저장한 시각이 아니라 접속 기록을 읽어들인 시각
                      </strong>
                      입니다.
                    </p>
                  </>
                }
                error={state.fieldErrors?.metricsExcludedHosts}
              >
                <TextArea
                  name="metricsExcludedHosts"
                  rows={4}
                  value={hosts}
                  onChange={(e) => setHosts(e.target.value)}
                  placeholder={"admin.example.com\nnginx.example.com"}
                />
              </Field>
            </div>

            {state.error ? (
              <Banner
                tone={"error"}
                title={<>저장 실패 — {state.error}</>}
                role="alert"
              ></Banner>
            ) : null}
            {state.ok && state.message ? (
              <Banner
                tone={"success"}
                title={<>{state.message}</>}
                role="status"
              ></Banner>
            ) : null}

            <footer className={adminStyles.foot}>
              <Button type="submit" variant="secondary" disabled={pending}>
                {pending ? "저장 중..." : "저장"}
              </Button>
            </footer>
          </div>
        </Card>
      </FormStack>
    </form>
  );
}
