import { Unstyled } from "@storybook/addon-docs/blocks";
import type { CSSProperties, ReactNode } from "react";
import { DocTable } from "../docs";
import { contrast, isColor, light, token } from "./tokens";
import "./foundations.css";
import { Icon } from "@repo/ui";

export type TokenRow = [name: string, usage: ReactNode];

/** Light and dark value of each token side by side, whatever the current theme is. */
export function Swatches({ rows }: { rows: TokenRow[] }) {
  return (
    <Unstyled>
      <div className="nwui-found-swatches">
        {rows.map(([name, usage]) => (
          <div key={name} className="nwui-found-swatch">
            <div className="nwui-found-chips">
              {(["light", "dark"] as const).map((theme) => {
                const value = token(name, theme);
                return (
                  <div key={theme} className="nwui-found-chip" style={{ background: token("--bg-canvas", theme) }}>
                    {/* The value is read from tokens.css, not written here. */}
                    <span className="nwui-found-chip-fill" style={{ background: value, borderColor: token("--line-strong", theme) }} aria-hidden />
                    <span className="nwui-found-chip-label" style={{ color: token("--text-2", theme) }}>
                      {theme === "light" ? "라이트" : "다크"} <code>{value}</code>
                    </span>
                  </div>
                );
              })}
            </div>
            <code className="nwui-found-name">{name}</code>
            <span className="nwui-found-usage">{usage}</span>
          </div>
        ))}
      </div>
    </Unstyled>
  );
}

/** Pass/fail against a WCAG threshold, in words so it does not rely on colour. */
function Ratio({ value, min }: { value: number; min: number }) {
  const pass = value >= min;
  return (
    <span className={pass ? "nwui-found-pass" : "nwui-found-fail"}>
      <span className="nwui-found-nowrap">{value.toFixed(2)}:1</span>{" "}
      <span className="nwui-found-nowrap"><Icon name={pass ? "check" : "close"} size={14} /> {pass ? "충족" : "미달"}</span>
    </span>
  );
}

export type Pair = [label: ReactNode, foreground: string, background: string, min: number];

/** Contrast measured from the token values of both themes. */
export function ContrastTable({ pairs }: { pairs: Pair[] }) {
  return (
    <DocTable
      columns={["조합", "기준", "라이트", "다크"]}
      rows={pairs.map(([label, fg, bg, min]) => [
        label,
        `${min}:1`,
        <Ratio value={contrast(token(fg), token(bg))} min={min} />,
        <Ratio value={contrast(token(fg, "dark"), token(bg, "dark"))} min={min} />,
      ])}
    />
  );
}

/** Every colour token, so a token added to tokens.css shows up even before a page describes it. */
export const colorTokenNames = Object.keys(light).filter((name) => isColor(token(name)) && isColor(token(name, "dark")));

/** Declared values of one CSS property across the product, marking those outside the design scale. */
export function UsageTable({
  rows,
  inScale,
  note,
}: {
  rows: [string, number][];
  inScale: (value: string) => boolean;
  note?: (value: string) => ReactNode;
}) {
  return (
    <DocTable
      columns={["값", "선언 수", "디자인 기준"]}
      rows={rows.map(([value, count]) => [
        <code>{value}</code>,
        String(count),
        inScale(value) ? "기준 안" : <span>기준 밖{note ? <> — {note(value)}</> : null}</span>,
      ])}
    />
  );
}

/** A labelled box for layout and spacing demos. */
export function Box({ children, style }: { children?: ReactNode; style?: CSSProperties }) {
  return (
    <div className="nwui-found-box" style={style}>
      {children}
    </div>
  );
}
