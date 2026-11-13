"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import {
  Button,
  EmptyState,
  Icon,
  Surface,
  FilterChips,
  SearchField,
  StatusBadge,
  Table,
  Td,
  Th,
  Tr,
  buttonClassName,
} from "@repo/ui";
import type { NodeFormAction } from "@/lib/node-action-state";
import type { ServerOpsRowVM } from "./server-status";
import { ServerToggle } from "./server-toggle";
import styles from "@/app/(app)/reverse-proxy/reverse-proxy.module.css";

/**
 * 서버 표(Operations): 상태 칩 + 검색 + 한 벌의 표. 좁은 화면에서는 같은 행이 요약 줄로 접힌다
 * (`@repo/ui` Table 의 `cell` 역할) — 행마다 폼과 링크는 한 번만 있다.
 *
 * 페이지(서버 컴포넌트)가 이미 전체 행을 읽어오므로 필터는 클라이언트에서 한다 — 서버 왕복도, 쿼리
 * 파라미터도 없이 입력 즉시 반응한다.
 *
 * **`basePath` 로 행 링크가 결정된다.** 로컬은 `/reverse-proxy`, 노드는 `/nodes/<id>/reverse-proxy` —
 * 꼬리가 같아서 치환이 한 번으로 끝난다. 활성 스위치와 「수정」은 **쓰기 권한이 있을 때만**
 * (`write` 가 있을 때) 그린다.
 */
export type { ServerRowVM } from "./server-vm";

type StatusFilter = "all" | "active" | "disabled" | "attention";

const needsAttention = (row: ServerOpsRowVM) =>
  row.health.tone === "error" || row.health.tone === "warning";

function matchesStatus(row: ServerOpsRowVM, status: StatusFilter): boolean {
  if (status === "active") return row.enabled;
  if (status === "disabled") return !row.enabled;
  if (status === "attention") return needsAttention(row);
  return true;
}

/** `server_name` 은 공백으로 구분된 여러 도메인일 수 있다 — 첫 이름을 보이고 나머지는 개수로. */
function names(serverName: string): { primary: string; extra: number } {
  const all = serverName.split(/\s+/).filter(Boolean);
  return { primary: all[0] ?? serverName, extra: Math.max(0, all.length - 1) };
}

export interface ServerTableWrite {
  /** 행 스위치가 보낼 명령(상세 화면의 토글과 같은 것). */
  toggleAction: NodeFormAction;
}

export function ServerTable({
  servers,
  basePath,
  write,
}: {
  servers: ServerOpsRowVM[];
  basePath: string;
  write: ServerTableWrite | null;
}) {
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<StatusFilter>("all");
  const q = query.trim().toLowerCase();

  const filtered = useMemo(
    // server_name 은 공백으로 구분된 다중 도메인일 수 있어 단순 substring 이 자연히 커버한다.
    // targetLabel 까지 이어 붙여 도메인으로도, 대상으로도 같은 입력창에서 찾는다.
    () =>
      servers.filter(
        (s) =>
          matchesStatus(s, status) &&
          (!q ||
            `${s.serverName} ${s.listen} ${s.targetLabel}`
              .toLowerCase()
              .includes(q)),
      ),
    [servers, status, q],
  );
  const count = (f: StatusFilter) =>
    servers.filter((s) => matchesStatus(s, f)).length;

  if (servers.length === 0) {
    return (
      <Surface flush>
        <EmptyState
          title="등록된 서버가 없습니다"
          description="도메인과 대상 주소를 정하면 nginx 설정이 만들어집니다."
          action={
            write ? (
              <Link
                href={`${basePath}/new`}
                className={buttonClassName({ variant: "secondary", size: "sm" })}
              >
                새 서버
              </Link>
            ) : undefined
          }
        />
      </Surface>
    );
  }

  return (
    <Surface
      flush
      toolbar={
        <div className={styles.opsToolbar}>
          <FilterChips
            label="상태 필터"
            value={status}
            onChange={(v) => setStatus(v as StatusFilter)}
            options={[
              { value: "all", label: "전체", count: count("all") },
              { value: "active", label: "활성", count: count("active") },
              { value: "disabled", label: "비활성", count: count("disabled") },
              {
                value: "attention",
                label: "확인 필요",
                count: count("attention"),
              },
            ]}
          />
          <SearchField
            size="sm"
            label="서버 검색"
            value={query}
            onChange={setQuery}
            placeholder="도메인 · 포트 · 대상"
          />
          <p className={styles.opsCount} aria-live="polite">
            {filtered.length}/{servers.length}
          </p>
        </div>
      }
    >
      {filtered.length === 0 ? (
        <EmptyState
          title="조건에 맞는 서버가 없습니다"
          action={
            <Button
              size="sm"
              variant="secondary"
              onClick={() => {
                setQuery("");
                setStatus("all");
              }}
            >
              필터 초기화
            </Button>
          }
        />
      ) : (
        <Table label="리버스 프록시 서버">
          <thead>
            <tr>
              <Th>상태</Th>
              <Th>서버</Th>
              <Th align="end">포트</Th>
              <Th>대상</Th>
              <Th>TLS</Th>
              {write ? (
                <>
                  <Th>활성</Th>
                  <Th>
                    <span className={styles.srOnly}>작업</span>
                  </Th>
                </>
              ) : null}
            </tr>
          </thead>
          <tbody>
            {filtered.map((s) => {
              const { primary, extra } = names(s.serverName);
              return (
                <Tr key={s.id} pending={s.pending}>
                  <Td cell="status">
                    <StatusBadge tone={s.health.tone}>
                      {s.health.label}
                    </StatusBadge>
                  </Td>
                  <Td
                    cell="primary"
                    wrap
                    icon={<Icon name="server" size={16} />}
                    secondary={s.health.reason || undefined}
                  >
                    <span className={styles.opsName}>
                      <Link
                        href={`${basePath}/${s.id}`}
                      >
                        {primary}
                      </Link>
                      {extra ? (
                        <span className={styles.opsMuted}> +{extra}</span>
                      ) : null}
                      {s.pending ? (
                        <StatusBadge
                          tone="info"
                          variant="outline"
                          size="sm"
                          dot={false}
                        >
                          미적용
                        </StatusBadge>
                      ) : null}
                    </span>
                  </Td>
                  <Td numeric label="포트">
                    {s.listen}
                  </Td>
                  {s.targets.length === 1 && s.targets[0]!.path === null ? (
                    <Td mono truncate label="대상" title={s.targetLabel}>
                      {s.targetLabel}
                    </Td>
                  ) : (
                    // location 마다 한 줄: 경로와 그 location 의 실제 대상. 잘라 내지 않고 줄바꿈한다.
                    <Td
                      mono
                      wrap
                      label="대상"
                      title={s.targetLabel}
                      className={styles.opsTargetsCell}
                    >
                      <ul className={styles.opsTargets}>
                        {s.targets.map((t, index) => (
                          <li key={index} className={styles.opsTarget}>
                            <span className={styles.opsTargetPath}>
                              {t.path}
                            </span>
                            {t.missingUpstream ? (
                              <StatusBadge
                                tone="warning"
                                variant="outline"
                                size="sm"
                                dot={false}
                              >
                                Upstream 없음
                              </StatusBadge>
                            ) : (
                              <span>{t.label}</span>
                            )}
                          </li>
                        ))}
                      </ul>
                    </Td>
                  )}
                  <Td label="TLS" tone={s.tls.tone === "neutral" ? undefined : s.tls.tone}>
                    {s.tls.label}
                  </Td>
                  {write ? (
                    <>
                      <Td cell="action">
                        <ServerToggle
                          id={s.id}
                          serverName={primary}
                          enabled={s.enabled}
                          action={write.toggleAction}
                        />
                      </Td>
                      <Td cell="action">
                        <Link
                          href={`${basePath}/${s.id}/edit`}
                          className={buttonClassName({
                            variant: "ghost",
                            size: "sm",
                            className: styles.opsRowAction,
                          })}
                          title="수정"
                          aria-label={`${primary} 수정`}
                        >
                          <Icon name="pencil" size={16} />
                        </Link>
                      </Td>
                    </>
                  ) : null}
                </Tr>
              );
            })}
          </tbody>
        </Table>
      )}
    </Surface>
  );
}
