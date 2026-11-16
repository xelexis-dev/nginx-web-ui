import type {
  HTMLAttributes,
  ReactNode,
  TdHTMLAttributes,
  ThHTMLAttributes,
} from "react";
import type { Tone } from "./tone";
import styles from "./table.module.css";

/**
 * A data table in one DOM for every width (DESIGN.md "data-table", Operations). Wide: cells pad
 * 8 × 12 with rows at least 36px (they grow with their content), a hairline header, numbers
 * right-aligned. At 768px and below the same rows reflow into summary blocks by each cell's role, so
 * per-row links and forms exist once:
 *   status · primary · action   on the first line,
 *   meta · meta · …             on the next (small, muted, dot-separated),
 *   hide-narrow                 not shown.
 * A cell without a role counts as meta. There the column header is hidden, so a value that means
 * nothing on its own (a date, a count) takes a `label`: it shows before the value instead of the dot.
 * A row with no data rows uses one cell spanning every column (`colSpan`), e.g. an `EmptyState`.
 */

export interface TableProps {
  /** Names the scroll region (and the table) for screen readers. */
  label: string;
  /** Comfortable rows for small lists; content may still grow beyond the 44px minimum. */
  density?: "dense" | "comfortable";
  layout?: "auto" | "fixed";
  narrow?: "summary" | "stacked";
  children: ReactNode;
}

export function Table({
  label,
  children,
  density = "dense",
  layout = "auto",
  narrow = "summary",
}: TableProps) {
  return (
    <div className={styles.wrap} role="region" aria-label={label} tabIndex={0}>
      <table
        className={styles.table}
        aria-label={label}
        data-density={density}
        data-layout={layout}
        data-narrow={narrow}
      >
        {children}
      </table>
    </div>
  );
}

export interface ThProps extends Omit<
  ThHTMLAttributes<HTMLTableCellElement>,
  "align"
> {
  /** Text alignment: "end" for numeric columns. */
  align?: "start" | "end";
}

export function Th({
  align = "start",
  scope = "col",
  className,
  ...rest
}: ThProps) {
  return (
    <th scope={scope} data-align={align} className={className} {...rest} />
  );
}

export interface TdProps extends TdHTMLAttributes<HTMLTableCellElement> {
  /** Role in the narrow layout. Default "meta". */
  cell?: "primary" | "status" | "meta" | "action" | "hide-narrow";
  numeric?: boolean;
  mono?: boolean;
  /** One line with an ellipsis (wide screens). Put the full text in `title` if it matters. */
  truncate?: boolean;
  /** Long values (URLs, tokens) wrap instead of widening the table (wide screens; narrow always wraps). */
  wrap?: boolean;
  /**
   * The column's name, shown before the value in the narrow layout where the header is hidden (e.g.
   * "만료"). For sight only: screen readers keep the column header. Leave it out for cells that
   * explain themselves (a name, a status badge, an action).
   */
  label?: string;
  /** Colours the text, e.g. an error rate over its threshold. */
  tone?: Tone;
  /** Optional decorative icon beside the existing cell content. */
  icon?: ReactNode;
  /** Optional supporting line; the column header and cell semantics remain intact. */
  secondary?: ReactNode;
}

export function Td({
  cell = "meta",
  numeric,
  mono,
  truncate,
  wrap,
  label,
  tone,
  icon,
  secondary,
  children,
  ...rest
}: TdProps) {
  return (
    <td
      data-cell={cell}
      data-numeric={numeric ? "" : undefined}
      data-mono={mono ? "" : undefined}
      data-truncate={truncate ? "" : undefined}
      data-wrap={wrap ? "" : undefined}
      data-label={label || undefined}
      data-tone={tone}
      {...rest}
    >
      {icon != null || secondary != null ? (
        <span className={styles.cellContent}>
          {icon != null ? (
            <span className={styles.cellIcon} aria-hidden="true">
              {icon}
            </span>
          ) : null}
          <span className={styles.cellText}>
            <span className={styles.primaryText}>{children}</span>
            {secondary != null ? (
              <span className={styles.secondary}>{secondary}</span>
            ) : null}
          </span>
        </span>
      ) : (
        children
      )}
    </td>
  );
}

export interface TrProps extends HTMLAttributes<HTMLTableRowElement> {
  /** A saved change not yet applied to nginx: the row is marked. Say so in text too. */
  pending?: boolean;
  current?: boolean;
}

export function Tr({ pending, current, ...rest }: TrProps) {
  return (
    <tr
      data-state={pending ? "pending" : undefined}
      data-current={current ? "" : undefined}
      {...rest}
    />
  );
}
