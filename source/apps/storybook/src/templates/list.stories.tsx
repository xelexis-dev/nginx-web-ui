import { TemplatePreview } from "./preview";
// Template: a list page (Storybook only). The contract a list screen follows, built from @repo/ui
// alone with sample values: header with the create action, one flush card with a filter strip and
// the table, and an empty state in the table's place. Nothing is saved.
import {
  Button,
  EmptyState,
  FilterChips,
  Icon,
  PageHeader,
  SearchField,
  SectionNav,
  StatusBadge,
  Surface,
  Facts,
  Table,
  Td,
  Th,
  Tr,
  buttonClassName,
  type Tone,
} from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { productFrame, productViewports } from "../product/frame";
import styles from "./templates.module.css";
import { PROXY_TABS } from "@/app/(app)/nav-links";
import Link from "../product/adapters/next-link";

interface Row {
  id: string;
  name: string;
  status: { tone: Tone; label: string };
  used: boolean;
  targets: number;
  servers: string;
  changed: string;
  pending?: boolean;
}

const ROWS: Row[] = [
  {
    id: "u1",
    name: "app_pool",
    status: { tone: "success", label: "사용 중" },
    used: true,
    targets: 3,
    servers: "app.example.test 외 1",
    changed: "2026. 9. 27. 오전 10:02",
  },
  {
    id: "u2",
    name: "api_v2",
    status: { tone: "success", label: "사용 중" },
    used: true,
    targets: 2,
    servers: "api.example.test",
    changed: "2026. 9. 26. 오후 6:40",
    pending: true,
  },
  {
    id: "u3",
    name: "legacy-backend-pool-for-the-old-billing-system",
    status: { tone: "neutral", label: "미사용" },
    used: false,
    targets: 1,
    servers: "—",
    changed: "2026. 8. 30. 오전 9:15",
  },
];

function ListTemplate({
  rows,
  cards = false,
}: {
  rows: Row[];
  cards?: boolean;
}) {
  const [filter, setFilter] = useState("all");
  const [query, setQuery] = useState("");
  const count = (f: string) =>
    rows.filter((r) => f === "all" || (f === "used") === r.used).length;
  const shown = rows.filter(
    (r) =>
      (filter === "all" || (filter === "used") === r.used) &&
      r.name.includes(query.trim()),
  );
  return (
    <TemplatePreview className={styles.page}>
      <PageHeader
        title="업스트림"
        description="서버가 요청을 넘길 대상 묶음입니다."
        actions={
          <a href="#new" className={buttonClassName()}>
            <Icon name="plus" />새 업스트림
          </a>
        }
      />
      <SectionNav
        label="프록시"
        items={PROXY_TABS}
        currentPath="/reverse-proxy/upstreams"
        renderLink={(props) => <Link {...props} />}
      />
      <Surface>
        <Facts
          columns={2}
          items={[
            { term: "업스트림", value: rows.length },
            { term: "사용 중", value: count("used") },
          ]}
        />
      </Surface>
      <Surface
        flush
        toolbar={
          <>
            <FilterChips
              label="사용 여부"
              value={filter}
              onChange={setFilter}
              options={[
                { value: "all", label: "전체", count: count("all") },
                { value: "used", label: "사용 중", count: count("used") },
                { value: "unused", label: "미사용", count: count("unused") },
              ]}
            />
            <SearchField
              label="업스트림 검색"
              value={query}
              onChange={setQuery}
              placeholder="이름"
            />
            <p className={styles.count} aria-live="polite">
              {shown.length}/{rows.length}
            </p>
          </>
        }
      >
        {rows.length === 0 ? (
          <EmptyState
            title="업스트림이 없습니다"
            description="대상 서버 묶음을 만들면 리버스 프록시가 요청을 나눠 보낼 수 있습니다."
            action={
              <a
                href="#new"
                className={buttonClassName({
                  variant: "secondary",
                  size: "sm",
                })}
              >
                새 업스트림
              </a>
            }
          />
        ) : shown.length === 0 ? (
          <EmptyState
            title="조건에 맞는 업스트림이 없습니다"
            action={
              <Button
                size="sm"
                variant="secondary"
                onClick={() => {
                  setFilter("all");
                  setQuery("");
                }}
              >
                필터 초기화
              </Button>
            }
          />
        ) : cards ? (
          <div className={styles.cards}>
            {shown.map((r) => (
              <Surface
                key={r.id}
                title={
                  <a className={styles.cardLink} href={`#${r.id}`}>
                    {r.name}
                  </a>
                }
                description={r.servers}
              >
                <StatusBadge tone={r.status.tone}>{r.status.label}</StatusBadge>
                <Facts
                  columns={2}
                  items={[
                    { term: "대상", value: r.targets + "개" },
                    { term: "마지막 변경", value: r.changed },
                  ]}
                />
                <a
                  href={`#${r.id}-edit`}
                  className={buttonClassName({ variant: "ghost", size: "sm" })}
                >
                  수정
                </a>
              </Surface>
            ))}
          </div>
        ) : (
          <Table label="업스트림">
            <thead>
              <tr>
                <Th>상태</Th>
                <Th>이름</Th>
                <Th align="end">대상</Th>
                <Th>쓰는 서버</Th>
                <Th>마지막 변경</Th>
                <Th>
                  <span className={styles.srOnly}>작업</span>
                </Th>
              </tr>
            </thead>
            <tbody>
              {shown.map((r) => (
                <Tr key={r.id} pending={r.pending}>
                  <Td cell="status">
                    <StatusBadge tone={r.status.tone}>
                      {r.status.label}
                    </StatusBadge>
                  </Td>
                  <Td cell="primary" wrap>
                    <a href={`#${r.id}`}>{r.name}</a>
                    {r.pending ? " · 미적용" : null}
                  </Td>
                  <Td numeric label="대상">
                    {r.targets}개
                  </Td>
                  <Td label="쓰는 서버">{r.servers}</Td>
                  <Td label="변경">{r.changed}</Td>
                  <Td cell="action">
                    <a
                      href={`#${r.id}-edit`}
                      className={buttonClassName({
                        variant: "ghost",
                        size: "sm",
                      })}
                    >
                      수정
                    </a>
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Surface>
    </TemplatePreview>
  );
}

const meta = {
  id: "templates-list",
  title: "Templates/목록",
  component: ListTemplate,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    productFrame(() => ({
      path: "/reverse-proxy/upstreams",
      label: "업스트림",
      role: "editor",
    })),
  ],
  args: { rows: ROWS },
} satisfies Meta<typeof ListTemplate>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = { name: "목록" };
export const Empty: Story = { name: "처음(행 없음)", args: { rows: [] } };
export const Narrow: Story = {
  name: "목록 · 390",
  globals: { viewport: { value: "p390", isRotated: false } },
};

export const Cards: Story = { name: "카드 목록", args: { cards: true } };
export const Dark: Story = { globals: { theme: "dark" } };
