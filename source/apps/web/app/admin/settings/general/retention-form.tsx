"use client";

import { useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import type {
  ApiErrorBody,
  RetentionCollectionView,
  RetentionEstimates,
  RetentionSettingsSaved,
  RetentionSettingsView,
} from "@repo/contracts";
import {
  Banner, Button, Card, CardFoot, Facts, Field, FormStack, Modal, StatusBadge, TextInput,
  formFieldsClassName, modalTargetClassName,
} from "@repo/ui";
import { HistoryTime } from "@/components/history/history-time";
import { FormErrors } from "@/components/form-errors";
import resourceStyles from "@/components/resource-layout.module.css";
import adminStyles from "@/app/admin/admin-pages.module.css";
import { SUBMIT_TIMEOUT_MS } from "@/lib/api-transport";
import styles from "./retention.module.css";

const PATH = "/api/ui/v1/admin/settings/retention";
const COLLECTIONS = [
  { key: "audit", field: "auditRetentionDays", expected: "expectedAuditRetentionDays", label: "감사 로그", minimum: 90, floor: 10_000 },
  { key: "history", field: "historyRetentionDays", expected: "expectedHistoryRetentionDays", label: "적용 이력", minimum: 30, floor: 100 },
] as const;
const STATUS = {
  off: { label: "꺼짐", tone: "neutral" },
  waiting: { label: "첫 정리 대기", tone: "info" },
  running: { label: "정리 중", tone: "info" },
  done: { label: "정리 완료", tone: "success" },
  failed: { label: "정리 실패", tone: "error" },
} as const;

type Collection = (typeof COLLECTIONS)[number];
type Periods = Record<Collection["key"], string>;
type SavedPeriods = { props: RetentionSettingsView; periods?: Periods };
type Edit = { baseline: Periods; draft: Periods };
type Target = Collection & { previous: number | null; days: number };
type Confirmation = { targets: Target[]; estimates: RetentionEstimates; body: URLSearchParams };
type Feedback = { fields?: Record<string, string>; error?: string; saved?: "scheduled" | "direct" };

const number = (value: number) => value.toLocaleString("ko-KR");

export function needsRetentionConfirmation(previous: number | null, proposed: number | null) {
  return proposed !== null && (previous === null || proposed < previous);
}

async function request<T>(url: string, body?: URLSearchParams): Promise<T> {
  const response = await fetch(url, {
    method: body ? "PUT" : "GET",
    body,
    cache: "no-store",
    credentials: "same-origin",
    redirect: "error",
    headers: { Accept: "application/json" },
    signal: AbortSignal.timeout(SUBMIT_TIMEOUT_MS),
  });
  const value = await response.json() as T | ApiErrorBody;
  if (!response.ok) throw (value as ApiErrorBody).error;
  return value as T;
}

function feedbackFor(error: unknown): Feedback {
  if (typeof error === "object" && error !== null && "fieldErrors" in error && error.fieldErrors) {
    return { fields: error.fieldErrors as Record<string, string> };
  }
  if (typeof error === "object" && error !== null && "code" in error && "message" in error
    && typeof error.message === "string") return { error: error.message };
  return { error: "API 서버에 요청을 전달하지 못했습니다. 잠시 후 다시 시도하세요." };
}

function NextRun({ value }: { value: RetentionCollectionView }) {
  switch (value.status) {
    case "off": return "없음";
    case "waiting":
      return value.nextRunAt
        ? <>약 10분 뒤 (<HistoryTime iso={value.nextRunAt} precision="time" />쯤)</>
        : "약 10분 뒤";
    case "running": return <>약 1시간 안 · 약 {number(value.remaining)}건 남음</>;
    case "done": return "약 1시간 안 · 지울 기록이 있을 때";
    case "failed": return "약 1시간 안에 다시 시도";
  }
}

function periodsFromProps(values: RetentionSettingsView): Periods {
  return {
    audit: values.audit.days?.toString() ?? "",
    history: values.history.days?.toString() ?? "",
  };
}

export function RetentionForm({ values }: { values: RetentionSettingsView }) {
  const router = useRouter();
  const [edit, setEdit] = useState<Edit | null>(null);
  const [saved, setSaved] = useState<SavedPeriods>(() => ({ props: values }));
  if (saved.props !== values) setSaved({ props: values });
  const current = (saved.props === values ? saved.periods : undefined) ?? periodsFromProps(values);
  const baseline = edit?.baseline ?? current;
  const draft = edit?.draft ?? current;

  function changeDays(key: Collection["key"], value: string) {
    const next = { ...draft, [key]: value };
    const changed = COLLECTIONS.some(({ key }) => next[key] !== baseline[key]);
    setEdit(changed ? { baseline, draft: next } : null);
  }
  const [feedback, setFeedback] = useState<Feedback>({});
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const [pending, setPending] = useState(false);

  async function save(body: URLSearchParams) {
    setPending(true);
    setFeedback({});
    try {
      const result = await request<RetentionSettingsSaved>(PATH, body);
      setConfirmation(null);
      const audit = body.get("auditRetentionDays");
      const history = body.get("historyRetentionDays");
      setSaved((observed) => {
        const periods = observed.periods ?? periodsFromProps(observed.props);
        if (COLLECTIONS.some(({ key, expected }) => periods[key] !== body.get(expected))) return observed;
        return {
          props: observed.props,
          periods: { audit: audit ? String(Number(audit)) : "", history: history ? String(Number(history)) : "" },
        };
      });
      setEdit(null);
      setFeedback({ saved: result.schedulesCleanup ? "scheduled" : "direct" });
      router.refresh();
    } catch (error) {
      const failure = feedbackFor(error);
      if (failure.fields || (typeof error === "object" && error !== null && "code" in error && error.code === "conflict")) setConfirmation(null);
      setFeedback(failure);
    } finally {
      setPending(false);
    }
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;
    const fields: Record<string, string> = {};
    const body = new URLSearchParams();
    const query = new URLSearchParams();
    const targets: Target[] = [];
    for (const collection of COLLECTIONS) {
      const { key, field, label, minimum } = collection;
      const raw = draft[key].trim();
      const days = raw ? Number(raw) : null;
      if (days !== null && (!/^[0-9]+$/.test(raw) || days < minimum || days > 3650)) {
        fields[field] = `${label} 보존 기간은 ${minimum} 이상 3650 이하의 정수여야 합니다(비우면 무제한).`;
      }
      body.set(field, raw);
      const previous = baseline[key] === "" ? null : Number(baseline[key]);
      body.set(collection.expected, baseline[key]);
      if (days !== null && needsRetentionConfirmation(previous, days)) {
        query.set(field, raw);
        targets.push({ ...collection, previous, days });
      }
    }
    if (Object.keys(fields).length) {
      setFeedback({ fields });
      return;
    }
    if (targets.length === 0) {
      await save(body);
      return;
    }
    setPending(true);
    setFeedback({});
    try {
      const estimates = await request<RetentionEstimates>(`${PATH}/estimate?${query}`);
      setConfirmation({ targets, estimates, body });
    } catch (error) {
      setFeedback(feedbackFor(error));
    } finally {
      setPending(false);
    }
  }

  return (
    <form className={resourceStyles.form} onSubmit={submit}>
      <FormErrors errors={feedback.fields} />
      <FormStack>
        <Card
          id="admin-retention"
          title="기록 보관"
          info="감사 로그와 적용 이력을 얼마나 오래 보관할지 정합니다. 기본은 무제한입니다."
          errorCount={Object.keys(feedback.fields ?? {}).length}
          footer={<CardFoot><Button type="submit" variant="secondary" disabled={pending}>{pending ? "저장 중..." : "저장"}</Button></CardFoot>}
        >
          <div className={adminStyles.body}>
            <div className={formFieldsClassName({ className: styles.groups })}>
              {COLLECTIONS.map(({ key, field, label, minimum, floor }) => {
                const value = values[key];
                const { lastCleanup, oldestAt, status } = value;
                return (
                  <section key={key} className={styles.group} aria-labelledby={field + "-heading"}>
                    <h3 id={field + "-heading"}>{label}</h3>
                    <Field
                      label="보존 기간 (일)" htmlFor={field} error={feedback.fields?.[field]}
                      hint={`${minimum}–3650일. 비우면 무제한입니다. 가장 최근 ${number(floor)}건은 기간이 지나도 지우지 않습니다.`}
                    >
                      <TextInput name={field} inputMode="numeric" placeholder="비우면 무제한" value={draft[key]} readOnly={pending || confirmation !== null}
                        onChange={(event) => changeDays(key, event.target.value)} />
                    </Field>
                    <Facts items={[
                      { term: "상태", value: <StatusBadge tone={STATUS[status].tone}>{STATUS[status].label}</StatusBadge> },
                      { term: "기록 수", value: <>{number(value.count)}건</> },
                      { term: "가장 오래된 기록", value: oldestAt ? <HistoryTime iso={oldestAt} precision="date" /> : "—" },
                      { term: "마지막 정리", value: lastCleanup ? <><HistoryTime iso={lastCleanup.at} /> · {number(lastCleanup.deleted)}건</> : "한 번도 없음" },
                      { term: "다음 정리", value: <NextRun value={value} /> },
                    ]} />
                  </section>
                );
              })}
            </div>
            <div className={styles.help}>
              <p>보존 기간을 넣고 저장하면 약 10분 뒤 첫 정리를 하고, 그 뒤로 약 1시간마다 기간이 지난 기록을 나누어 지웁니다. <strong>지운 기록은 되돌릴 수 없고</strong>, 지운 적용 이력으로는 롤백할 수 없습니다.</p>
              <p>기록을 지워도 <strong>이미 확보된 디스크 공간은 즉시 반환되지 않고</strong> 새 기록에 재사용됩니다. 이 설정은 이 설치의 기록에만 적용됩니다. 노드의 기록은 각 노드에서 따로 정합니다.</p>
            </div>
            {feedback.error && !confirmation && <Banner tone="error" title={<>저장 실패 — {feedback.error}</>} />}
            {feedback.saved === "direct" && <Banner tone="success" title="보존 기간을 저장했습니다." />}
            {feedback.saved === "scheduled" && (
              <Banner tone="success" title="보존 기간을 저장했습니다 — 약 10분 뒤 첫 정리를 시작합니다.">
                그 전에 칸을 비우고 다시 저장하면 지우지 않습니다.
              </Banner>
            )}
            {COLLECTIONS.map(({ key, label }) => {
              const { status, lastFailure } = values[key];
              if (status !== "failed" || !lastFailure) return null;
              return (
                <Banner key={key} tone="error" title={`${label} 정리 실패`} output={lastFailure.message}>
                  <div className={styles.failureText}>
                    <p><HistoryTime iso={lastFailure.at} /> 정리가 오류로 중단됐습니다. 그 전에 지운 {number(lastFailure.deleted)}건은 지워진 그대로이고, 나머지 기록은 남아 있습니다.</p>
                    <p>약 1시간 안에 자동으로 다시 시도하므로 지금 할 일은 없습니다. 다음에도 실패하면 아래 오류와 컨테이너 로그를 확인하세요. 정리를 멈추려면 보존 기간을 비우고 저장하세요.</p>
                  </div>
                </Banner>
              );
            })}
          </div>
        </Card>
      </FormStack>
      <Modal
        open={confirmation !== null} dismissible={!pending} onClose={() => setConfirmation(null)}
        title="보존 기간 저장" icon="danger"
        description="저장하면 약 10분 뒤부터 보존 기간이 지난 기록을 지웁니다. 지운 기록은 되돌릴 수 없습니다."
        footer={(
          <>
            <Button type="button" variant="secondary" disabled={pending} onClick={() => setConfirmation(null)}>취소</Button>
            <Button type="button" variant="danger" disabled={pending} aria-busy={pending || undefined}
              onClick={() => confirmation && void save(confirmation.body)}>저장하고 정리 시작</Button>
          </>
        )}
      >
        {confirmation && <>
          <div className={styles.targets}>
            {confirmation.targets.map(({ key, label, days, previous, floor }) => {
              const estimate = confirmation.estimates[key];
              if (!estimate) return null;
              const date = <HistoryTime iso={estimate.cutoff} precision="date" />;
              return (
                <div className={modalTargetClassName} key={key}>
                  <span>{label} · {previous === null ? "꺼짐" : `${previous}일`}에서 {days}일로</span>
                  <strong>{estimate.deleted ? `약 ${number(estimate.deleted)}건을 지웁니다` : "지금 지울 기록은 없습니다"}</strong>
                  <span>{estimate.deleted ? <>{date} 이전 {key === "audit" ? "기록" : "이력"}이 대상입니다.</> : <>앞으로 {date} 이전이 되는 기록부터 지웁니다.</>} 가장 최근 {number(floor)}건은 날짜와 상관없이 남깁니다.</span>
                </div>
              );
            })}
          </div>
          <ul className={styles.details}>
            {confirmation.targets.some(({ key }) => key === "history") && <li>지운 적용 이력은 내용을 볼 수 없고 <strong>롤백에도 쓸 수 없습니다.</strong></li>}
            <li>저장하기 전에 <strong>백업</strong>을 권합니다.</li>
            <li>기록이 많으면 약 1시간마다 나누어 지웁니다.</li>
            {confirmation.targets.some(({ previous }) => previous === null) && <li>10분 안에 칸을 비우고 다시 저장하면 지우지 않습니다.</li>}
            {[...new Set(confirmation.targets.flatMap(({ previous }) => previous === null ? [] : [previous]))].map((previous) => (
              <li key={previous}>10분 안에 이전 값({previous}일)으로 다시 저장하면 지우지 않습니다.</li>
            ))}
          </ul>
          {feedback.error && <Banner tone="error" title={<>저장 실패 — {feedback.error}</>} />}
        </>}
      </Modal>
    </form>
  );
}
