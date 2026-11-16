"use client";

import { Fragment, useState } from "react";
import Link from "next/link";
import { Button, Icon, StatusBadge } from "@repo/ui";
import type { OperationsTarget } from "@repo/presentation/operations-target";
import type { ConfigDiff, ItemDiff } from "@/lib/config-diff";
import { flattenChanges, type FieldRow } from "@/lib/config-diff-flatten";
import {
  DIFF_GROUPS,
  DIFF_MARKERS,
  GROUP_TITLES,
  itemKind,
  serverLabel,
  sortedItems,
  type DiffGroup,
} from "@/lib/config-diff-summary";
import styles from "./config-diff-view.module.css";

export interface ConfigDiffViewProps {
  diff: ConfigDiff;
  variant?: "inline" | "panel";
  target?: OperationsTarget;
  canEdit?: boolean;
  onNavigate?: () => void;
  disabled?: boolean;
}
function Path({ path }: { path: string[] }) {
  return (
    <>
      {path.map((part, i) => (
        <Fragment key={i}>
          {i ? (
            <>
              <span aria-hidden className={styles.separator}>
                {" "}
                ›{" "}
              </span>
              <span className={styles.srOnly}>, </span>
            </>
          ) : null}
          {part}
        </Fragment>
      ))}
    </>
  );
}
function Value({ value, title }: { value: string | null; title?: string }) {
  return (
    <span className={value === null ? styles.missing : undefined} title={title}>
      {value ?? "없음"}
    </span>
  );
}
function FieldTable({ rows }: { rows: FieldRow[] }) {
  if (!rows.length) return null;
  return (
    <table className={styles.fields}>
      <thead>
        <tr>
          <th scope="col">설정</th>
          <th scope="col">이전</th>
          <th scope="col">이후</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => (
          <tr key={row.key}>
            <th scope="row">
              <Path path={row.path} />
            </th>
            <td data-l="이전">
              <Value value={row.before} title={row.beforeTitle} />
            </td>
            <td data-l="이후">
              <Value value={row.after} title={row.afterTitle} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
function FullSettings({ rows }: { rows: FieldRow[] }) {
  const section = (row: FieldRow) => {
    const first = row.path[0] ?? "";
    return first.startsWith("location ") ||
      [
        "TLS",
        "응답 헤더",
        "server 블록 디렉티브",
        "접근 제어",
        "실 IP 복원",
        "프록시 캐시",
        "gzip",
      ].includes(first)
      ? first
      : "기본";
  };
  const rank = (name: string) =>
    name.startsWith("location ")
      ? 2
      : [
          "기본",
          "TLS",
          "location",
          "응답 헤더",
          "server 블록 디렉티브",
          "접근 제어",
          "실 IP 복원",
          "프록시 캐시",
          "gzip",
        ].indexOf(name);
  const sections = [...new Set(rows.map(section))].sort(
    (a, b) => rank(a) - rank(b),
  );
  return (
    <div className={styles.full}>
      {sections.map((title) => (
        <section key={title}>
          <h4>{title}</h4>
          <dl>
            {rows
              .filter((row) => section(row) === title)
              .map((row) => (
                <div key={row.key}>
                  <dt>
                    <Path
                      path={
                        row.path[0] === title && row.path.length > 1
                          ? row.path.slice(1)
                          : row.path
                      }
                    />
                  </dt>
                  <dd>
                    <Value
                      value={row.after ?? row.before}
                      title={row.afterTitle ?? row.beforeTitle}
                    />
                    {row.changed ? (
                      <StatusBadge size="sm" dot={false} tone="neutral">
                        {row.before === null
                          ? "추가"
                          : row.after === null
                            ? "삭제"
                            : "변경"}
                      </StatusBadge>
                    ) : null}
                  </dd>
                </div>
              ))}
          </dl>
        </section>
      ))}
    </div>
  );
}
function DiffItem({
  item,
  group,
  lookups,
  target,
  canEdit,
  onNavigate,
  disabled,
}: Omit<ConfigDiffViewProps, "diff" | "variant"> & {
  item: ItemDiff;
  group: DiffGroup;
  lookups: Record<string, string>;
}) {
  const [expanded, setExpanded] = useState(false);
  const kind = itemKind(item),
    marker = DIFF_MARKERS[kind];
  const label =
    group === "servers"
      ? serverLabel(item.label)
      : { name: item.label, port: null };
  const rows = flattenChanges(group, item, lookups);
  const hasConfig = item.changes.some((change) => change.field === "config");
  const full = hasConfig ? flattenChanges(group, item, lookups, true) : [];
  const prefix =
    target?.kind === "node" ? `/nodes/${encodeURIComponent(target.id)}` : "";
  const route =
    group === "servers"
      ? "/reverse-proxy"
      : group === "upstreams"
        ? "/reverse-proxy/upstreams"
        : "/stream";
  return (
    <li className={styles.diffItem}>
      <span className={styles.marker} data-kind={kind}>
        <span aria-hidden>{marker.char}</span>
        <span className={styles.srOnly}>{marker.label}</span>
      </span>
      <div className={styles.itemContent}>
        <div className={styles.main}>
          <span className={styles.name}>{label.name}</span>
          <div className={styles.summary}>
            {kind === "skipped" ? (
              <>
                <strong className={styles.excluded}>적용에서 제외됨</strong>
                {item.reason ? (
                  <span className={styles.reason}>{item.reason}</span>
                ) : null}
                <span className={styles.consequence}>
                  {item.status === "changed"
                    ? "마지막으로 적용된 설정이 계속 사용됩니다."
                    : "이 항목은 지금 동작하지 않습니다."}
                  {canEdit && item.status !== "removed" ? (
                    <>
                      {" "}
                      {disabled ? (
                        <span aria-disabled="true">설정 수정</span>
                      ) : (
                        <Link
                          href={`${prefix}${route}/${encodeURIComponent(item.key)}/edit`}
                          onClick={onNavigate}
                        >
                          설정 수정
                        </Link>
                      )}
                    </>
                  ) : null}
                </span>
              </>
            ) : kind === "changed" ? (
              `설정 ${rows.length}곳 변경`
            ) : (
              `${kind === "added" ? "새로 추가" : "삭제"}${label.port ? ` · listen ${label.port}` : ""}`
            )}
          </div>
        </div>
        <FieldTable rows={rows} />
        {hasConfig ? (
          <div className={styles.expander}>
            <span>
              바뀌지 않은 설정 {full.filter((row) => !row.changed).length}개
            </span>
            <Button
              type="button"
              variant="ghost"
              size="xs"
              disabled={disabled}
              aria-expanded={expanded}
              onClick={() => setExpanded(!expanded)}
            >
              {expanded ? "전체 설정 접기" : "전체 설정 보기"}
              <span className={expanded ? styles.rotated : undefined}>
                <Icon name="chevronDown" size={12} />
              </span>
            </Button>
          </div>
        ) : null}
        {expanded ? <FullSettings rows={full} /> : null}
      </div>
    </li>
  );
}
/** Shared by pending reviews, viewer/badge dialogs and history snapshots. */
export function ConfigDiffView({
  diff,
  variant = "inline",
  ...props
}: ConfigDiffViewProps) {
  const lookups = Object.fromEntries(
    diff.upstreams.map((item) => [item.key, item.label]),
  );
  return (
    <div className={styles.diffGroups} data-variant={variant}>
      {DIFF_GROUPS.map((group) =>
        diff[group].length ? (
          <section key={group}>
            <h3 className={styles.groupTitle}>
              {GROUP_TITLES[group]}{" "}
              <span>
                {diff[group].length}
                <span className={styles.srOnly}>건</span>
              </span>
            </h3>
            <ul className={styles.diffList}>
              {sortedItems(diff[group]).map((item) => (
                <DiffItem
                  key={item.key}
                  item={item}
                  group={group}
                  lookups={lookups}
                  {...props}
                />
              ))}
            </ul>
          </section>
        ) : null,
      )}
    </div>
  );
}
