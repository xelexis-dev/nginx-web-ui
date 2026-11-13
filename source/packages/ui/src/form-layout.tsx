import {
  cloneElement,
  isValidElement,
  type ReactElement,
  type ReactNode,
} from "react";
import { Icon } from "./icon";
import { InfoTip } from "./info-tip";
import styles from "./form-layout.module.css";

/**
 * Server-safe form structure (DESIGN.md "form-set", Operations): titled form sections, Field, which labels one control and wires its hint and error, and FieldGroup,
 * which names several controls together (a fieldset and its legend).
 */

export interface FormCardOptions {
  /**
   * 2: the fields flow in two columns (gap 20; one column at 768px and below); a child with
   * `data-wide` (a `Field wide`, a textarea row, the action row, a form error) spans both. Default 1: one
   * column, 20 between rows (a stacked form whose field block uses `formFieldsClassName`).
   */
  columns?: 1 | 2;
  className?: string;
}

/**
 * The card a page form sits in (surface, hairline border, large radius, surface padding), as a class
 * for the `<form>` itself — like `buttonClassName`, so the form keeps its own element, action and
 * handlers and no wrapper is added.
 */
export function formCardClassName({
  columns = 1,
  className,
}: FormCardOptions = {}): string {
  return [
    styles.formCard,
    columns === 2 ? styles.columns2 : undefined,
    className,
  ]
    .filter(Boolean)
    .join(" ");
}

/**
 * A form's field block without a card: inside a stacked form card, or a form inside a titled `Surface`.
 * columns 2 (default): the same grid as `formCardClassName({ columns: 2 })`. 1: one column, 20 between rows.
 */
export function formFieldsClassName({
  columns = 2,
  className,
}: FormCardOptions = {}): string {
  return [columns === 2 ? styles.columns2 : styles.columns1, className]
    .filter(Boolean)
    .join(" ");
}

export interface FormSectionProps {
  info?: ReactNode;
  id: string;
  title: ReactNode;
  description?: ReactNode;
  headingLevel?: 2 | 3;
  /**
   * 2: the fields flow in two columns (gap 20; one column at 768px and below). A
   * `Field wide`, or any direct child with `data-wide`, spans both (a textarea, the action row, a
   * form error). Default 1: the body lays nothing out, as before.
   */
  columns?: 1 | 2;
  children: ReactNode;
}

export interface CardProps {
  id: string;
  title: ReactNode;
  description?: ReactNode;
  info?: ReactNode;
  actions?: ReactNode;
  headingLevel?: 2 | 3;
  errorCount?: number;
  off?: boolean;
  loading?: boolean;
  children?: ReactNode;
  footer?: ReactNode;
}

export function Card({
  id,
  title,
  description,
  info,
  actions,
  headingLevel = 2,
  errorCount = 0,
  off = false,
  loading = false,
  children,
  footer,
}: CardProps) {
  const Heading = headingLevel === 3 ? "h3" : "h2";
  return (
    <section
      id={id}
      className={styles.card}
      aria-labelledby={`${id}-heading`}
      aria-busy={loading || undefined}
      data-error={errorCount > 0 ? "" : undefined}
    >
      <header className={styles.cardHead}>
        <div className={styles.sectionTitle}>
          <div className={styles.titleRow}>
            <Heading id={`${id}-heading`} tabIndex={-1}>
              {title}
            </Heading>
            {info ? (
              <InfoTip
                label={typeof title === "string" ? `${title} 설명` : "설명"}
              >
                {info}
              </InfoTip>
            ) : null}
            {errorCount > 0 ? (
              <span className={styles.errorTag}>오류 {errorCount}</span>
            ) : null}
          </div>
          {description ? <p>{description}</p> : null}
        </div>
        {actions ? <div className={styles.cardActions}>{actions}</div> : null}
      </header>
      {children !== undefined || loading ? (
        <div className={styles.cardBody} hidden={off}>
          {loading ? (
            <div
              className={styles.skeleton}
              role="status"
              aria-label="불러오는 중"
            />
          ) : (
            children
          )}
        </div>
      ) : null}
      {footer}
    </section>
  );
}

export function CardFoot({ children }: { children: ReactNode }) {
  return <div className={styles.cardFoot}>{children}</div>;
}

export function FormSection({
  columns = 1,
  children,
  ...props
}: FormSectionProps) {
  return (
    <Card {...props}>
      <div
        className={styles.sectionBody}
        data-columns={columns === 2 ? "2" : undefined}
      >
        {children}
      </div>
    </Card>
  );
}

export function FormStack({ children }: { children: ReactNode }) {
  return <div className={styles.formStack}>{children}</div>;
}

export function Blocks({ children }: { children: ReactNode }) {
  return <div className={styles.blocks}>{children}</div>;
}

export interface BlockProps {
  id: string;
  number: number;
  title: ReactNode;
  tag?: ReactNode;
  summary?: ReactNode;
  /** Caller supplies labelled move, collapse and delete buttons; state belongs to the form. */
  actions?: ReactNode;
  expanded?: boolean;
  errorCount?: number;
  children: ReactNode;
}

export function Block({
  id,
  number,
  title,
  tag,
  summary,
  actions,
  expanded = true,
  errorCount = 0,
  children,
}: BlockProps) {
  return (
    <section
      id={id}
      className={styles.block}
      aria-labelledby={`${id}-heading`}
      data-error={errorCount > 0 ? "" : undefined}
    >
      <header className={styles.blockHead}>
        <span className={styles.blockNumber}>{number}</span>
        <h3 id={`${id}-heading`} className={styles.blockTitle}>
          {title}
        </h3>
        {tag}
        {errorCount > 0 ? (
          <span className={styles.errorTag}>오류 {errorCount}</span>
        ) : null}
        {summary ? (
          <span className={styles.blockSummary}>{summary}</span>
        ) : null}
        {actions ? <div className={styles.blockActions}>{actions}</div> : null}
      </header>
      <div id={`${id}-body`} className={styles.blockBody} hidden={!expanded}>
        {children}
      </div>
    </section>
  );
}

export interface FormBarProps {
  note?: ReactNode;
  children: ReactNode;
}
export function FormBar({ note, children }: FormBarProps) {
  return (
    <footer className={styles.formBar}>
      {note ? <div className={styles.formNote}>{note}</div> : null}
      <div className={styles.formActions}>{children}</div>
    </footer>
  );
}

/** What Field gives its control. Spread it onto the input, textarea or Select. */
export interface FieldControlProps {
  id: string;
  "aria-describedby"?: string;
  "aria-invalid"?: true;
  "aria-required"?: true;
}

export interface FieldProps {
  info?: ReactNode;
  label: ReactNode;
  /** The control's id. */
  htmlFor: string;
  hint?: ReactNode;
  error?: ReactNode;
  required?: boolean;
  optional?: boolean;
  /** Extra ids the control is described by (e.g. a server-side error rendered elsewhere). */
  describedBy?: string;
  /** In a two-column `FormSection`, span both columns (a textarea, a long value). */
  wide?: boolean;
  span?: 3 | 4 | 6 | 8 | 12;
  /** Label and hint on the left, one control on the right. */
  layout?: "stacked" | "inline";
  /**
   * The control. An element is cloned with the FieldControlProps at render; its own
   * aria-describedby is merged, and its own aria-invalid / aria-required are kept unless Field has
   * an error / required. A function receives them to spread itself.
   */
  children:
    | ReactElement<Record<string, unknown>>
    | ((control: FieldControlProps) => ReactNode);
}

export function Field({
  info,
  label,
  htmlFor,
  hint,
  error,
  required = false,
  optional = false,
  describedBy,
  wide = false,
  span,
  layout = "stacked",
  children,
}: FieldProps) {
  const infoId = info ? `${htmlFor}-info` : undefined;
  const hintId = hint ? `${htmlFor}-hint` : undefined;
  const errorId = error ? `${htmlFor}-error` : undefined;
  const merge = (own?: unknown) =>
    [
      typeof own === "string" ? own : undefined,
      describedBy,
      hintId,
      errorId,
      infoId,
    ]
      .filter(Boolean)
      .join(" ") || undefined;
  const control: FieldControlProps = {
    id: htmlFor,
    "aria-describedby": merge(),
    "aria-invalid": error ? true : undefined,
    "aria-required": required ? true : undefined,
  };
  const rendered =
    typeof children === "function"
      ? children(control)
      : isValidElement(children)
        ? cloneElement(children, {
            ...control,
            "aria-describedby": merge(children.props["aria-describedby"]),
            "aria-invalid": error ? true : children.props["aria-invalid"],
            "aria-required": required ? true : children.props["aria-required"],
          })
        : children;
  const labelElement = (
    <label className={styles.label} htmlFor={htmlFor}>
      {label}
      {optional && !required ? (
        <span className={styles.optional}>(선택)</span>
      ) : null}
    </label>
  );
  return (
    <div
      className={styles.field}
      data-wide={wide ? "" : undefined}
      data-span={wide ? 12 : span}
      data-layout={layout === "inline" ? "inline" : undefined}
    >
      {info ? (
        <div className={styles.labelRow}>
          {labelElement}
          <InfoTip
            label={typeof label === "string" ? `${label} 설명` : "설명"}
            id={infoId}
          >
            {info}
          </InfoTip>
        </div>
      ) : (
        labelElement
      )}
      {layout === "inline" ? (
        <div className={styles.inlineControl}>{rendered}</div>
      ) : (
        rendered
      )}
      {hint ? (
        // A div, so a long hint can be several <p>s and still be one description.
        <div className={styles.hint} id={hintId}>
          {hint}
        </div>
      ) : null}
      {error ? (
        <p className={styles.error} id={errorId}>
          <Icon name="circleAlert" size={14} />
          {error}
        </p>
      ) : null}
    </div>
  );
}

export interface FieldGroupProps {
  /** The fieldset's id; its hint and error take `${id}-hint` and `${id}-error`. */
  id: string;
  /** The group's visible name (the legend). */
  legend: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  /** Marks the legend with the required state (controls carry the required attribute). The controls carry their own `required`. */
  required?: boolean;
  /** In a two-column `FormSection`, span both columns. */
  wide?: boolean;
  /** Optional field grid. Omitted keeps the existing children layout. */
  columns?: 1 | 2;
  /** The controls: a SegmentedControl, a set of Checkboxes, a repeated row. Each labels itself. */
  children: ReactNode;
}

/**
 * Several controls answered together, named once: a fieldset whose legend looks like a Field label,
 * with the same hint and error below. The hint and error describe the whole group (aria-describedby
 * on the fieldset). One control with its own label is a `Field`; one checkbox is a `Checkbox`.
 */
export function FieldGroup({
  id,
  legend,
  hint,
  error,
  required: _required = false,
  wide = false,
  columns,
  children,
}: FieldGroupProps) {
  const hintId = hint ? `${id}-hint` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  return (
    <fieldset
      id={id}
      className={styles.group}
      data-wide={wide ? "" : undefined}
      aria-describedby={
        [hintId, errorId].filter(Boolean).join(" ") || undefined
      }
    >
      <legend className={styles.label}>{legend}</legend>
      {columns ? (
        <div className={styles.groupBody} data-columns={columns}>
          {children}
        </div>
      ) : (
        children
      )}
      {hint ? (
        // A div, so a long hint can be several <p>s and still be one description.
        <div className={styles.hint} id={hintId}>
          {hint}
        </div>
      ) : null}
      {error ? (
        <p className={styles.error} id={errorId}>
          <Icon name="circleAlert" size={14} />
          {error}
        </p>
      ) : null}
    </fieldset>
  );
}
