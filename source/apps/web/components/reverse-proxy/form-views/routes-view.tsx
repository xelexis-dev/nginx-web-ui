"use client";
import {
  Icon,
  Banner,
  Card,
  Block,
  Blocks,
  Button,
  IconButton,
  StatusBadge,
} from "@repo/ui";
import type { ReactNode } from "react";
import {
  directiveSummary,
  routeRetryInfo,
} from "@repo/presentation/nginx/server-summary";
import type { LocationInput } from "../server-form-vm";
import formStyles from "@/components/reverse-proxy/server-form.module.css";
export function locationError(
  fieldErrors: Record<string, string> | undefined,
  index: number,
): string | undefined {
  const indexed = Object.entries(fieldErrors ?? {}).find(([key]) =>
    key.startsWith(`locations.${index}.`),
  );
  if (indexed) return indexed[1];
  const message = fieldErrors?.locations;
  const match = message?.match(/location\s+(\d+)/i);
  return match && Number(match[1]) === index + 1 ? message : undefined;
}
export function RoutesView({
  locations,
  upstreams,
  open,
  onToggle,
  onAdd,
  onRemove,
  onMove,
  converted,
  fieldErrors,
  editor,
}: {
  locations: LocationInput[];
  upstreams: { id: string; name: string }[];
  open: ReadonlySet<number>;
  onToggle: (index: number) => void;
  onAdd: () => void;
  onRemove: (index: number) => void;
  onMove: (index: number, offset: -1 | 1) => void;
  converted: boolean;
  fieldErrors?: Record<string, string>;
  editor: (index: number) => ReactNode;
}) {
  const unindexed =
    fieldErrors?.locations && !/location\s+\d+/i.test(fieldErrors.locations);
  const missing = (route: LocationInput) =>
    route.mode === "upstream" &&
    (!!route.upstreamError ||
      (!!route.upstreamId &&
        !upstreams.some((u) => u.id === route.upstreamId)));
  const count =
    locations.filter(
      (route, index) => locationError(fieldErrors, index) || missing(route),
    ).length + (unindexed ? 1 : 0);
  return (
    <Card
      id="server-form-routes-card"
      title="위치"
      info="어느 경로의 요청을 어디로 보낼지 정합니다."
      errorCount={count}
      actions={
        <Button type="button" variant="secondary" size="sm" onClick={onAdd}>
          <Icon name="plus" />
          경로 추가
        </Button>
      }
    >
      <div className={formStyles.pfCategory}>
        {converted ? (
          <Banner
            tone="info"
            title="저장하면 이 서버가 경로별 형식으로 바뀝니다"
          >
            <p>
              화면에는 이미 경로 / 하나로 보이며, 따로 할 일은 없습니다. nginx
              설정은 바뀌지 않습니다. / 는 지금과 같은 location / 블록이 됩니다.
            </p>
            {locations[0]?.mode === "upstream" ? (
              <p>
                Upstream{" "}
                {upstreams.find((u) => u.id === locations[0]?.upstreamId)
                  ?.name ?? ""}
                은 지금처럼 고유 ID로 연결됩니다. 이름을 바꾸면 이 경로도
                따라가고, 이 경로가 쓰는 동안에는 지울 수 없습니다.
              </p>
            ) : null}
            <p>저장된 디렉티브는 고치지 않으면 순서와 값 그대로 옮깁니다.</p>
          </Banner>
        ) : null}
        {unindexed ? (
          <Banner tone="error" role="alert" title={fieldErrors?.locations} />
        ) : null}
        <div role="group" aria-label="경로 목록">
          <Blocks>
            {locations.map((route, index) => {
              const error = locationError(fieldErrors, index) || missing(route);
              const target =
                route.mode === "direct"
                  ? route.proxyPass
                  : upstreams.find((u) => u.id === route.upstreamId)?.name;
              const ws = directiveSummary(
                route.directives,
                route.originalDirectives,
              ).webSocket;
              const overrides = [
                route.retryOtherInstances === true ? "재시도" : "",
                ws !== "꺼짐" ? `WebSocket ${ws}` : "",
                route.cacheEnabled !== undefined
                  ? `캐시 ${route.cacheEnabled ? "켜기" : "끄기"}`
                  : "",
                route.gzipEnabled !== undefined
                  ? `gzip ${route.gzipEnabled ? "켜기" : "끄기"}`
                  : "",
                route.ipAccess ? "IP 규칙" : "",
              ].filter(Boolean);
              const move = (offset: -1 | 1, root: HTMLElement | null) => {
                onMove(index, offset);
                requestAnimationFrame(() =>
                  root
                    ?.querySelector<HTMLButtonElement>(
                      `[data-route-toggle="${index + offset}"]`,
                    )
                    ?.focus(),
                );
              };
              return (
                <Block
                  key={index}
                  id={`route-block-${index}`}
                  number={index + 1}
                  title={
                    <span id={`route-editor-heading-${index}`} tabIndex={-1}>
                      {route.path || "(경로 입력)"}
                    </span>
                  }
                  tag={
                    <StatusBadge tone="neutral" dot={false}>
                      {route.mode === "upstream" ? "Upstream" : "proxy_pass"}
                    </StatusBadge>
                  }
                  summary={
                    <span className={formStyles.pfRouteSummary}>
                      {target ||
                        (missing(route) ? "Upstream 없음" : "보낼 곳 선택")}
                      {overrides.map((value) => (
                        <StatusBadge
                          key={value}
                          dot={false}
                          title={
                            value === "재시도"
                              ? routeRetryInfo(route).summary
                              : undefined
                          }
                          tone={
                            value.startsWith("WebSocket") ? "info" : "neutral"
                          }
                        >
                          {value}
                        </StatusBadge>
                      ))}
                    </span>
                  }
                  expanded={open.has(index)}
                  errorCount={error ? 1 : 0}
                  actions={
                    <>
                      <IconButton
                        size="sm"
                        label={`경로 ${index + 1} 위로 이동`}
                        disabled={index === 0}
                        onClick={(event) =>
                          move(
                            -1,
                            event.currentTarget.closest(
                              '[aria-label="경로 목록"]',
                            ),
                          )
                        }
                      >
                        <Icon name="arrowUp" />
                      </IconButton>
                      <IconButton
                        size="sm"
                        label={`경로 ${index + 1} 아래로 이동`}
                        disabled={index === locations.length - 1}
                        onClick={(event) =>
                          move(
                            1,
                            event.currentTarget.closest(
                              '[aria-label="경로 목록"]',
                            ),
                          )
                        }
                      >
                        <Icon name="arrowDown" />
                      </IconButton>
                      <IconButton
                        size="sm"
                        data-route-toggle={index}
                        label={`경로 ${index + 1} ${open.has(index) ? "접기" : "펼치기"}`}
                        aria-expanded={open.has(index)}
                        aria-controls={`route-block-${index}-body`}
                        onClick={() => onToggle(index)}
                      >
                        <span className={open.has(index) ? formStyles.pfExpandedChevron : undefined}><Icon name="chevronDown" /></span>
                      </IconButton>
                      <IconButton
                        size="sm"
                        variant="dangerQuiet"
                        label="이 경로 삭제"
                        aria-describedby={`route-editor-heading-${index}`}
                        disabled={locations.length <= 1}
                        onClick={() => onRemove(index)}
                      >
                        <Icon name="trash2" />
                      </IconButton>
                    </>
                  }
                >
                  {editor(index)}
                </Block>
              );
            })}
          </Blocks>
        </div>
      </div>
    </Card>
  );
}
