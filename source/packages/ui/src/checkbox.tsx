import { useId, type ComponentProps, type ReactNode } from "react";
import styles from "./checkbox.module.css";
import { Icon } from "./icon";

/**
 * One yes/no choice with its own words: the browser's own `<input type="checkbox">` inside its label
 * (DESIGN.md "form-set"). Everything native passes through — name, value, checked or defaultChecked,
 * onChange, required, disabled, form, ref — so forms submit it as they always have, and Space toggles it.
 * An on/off setting with no wording of its own is `Switch`; several checkboxes answered together go in
 * a `FieldGroup`.
 */
export type CheckboxProps = Omit<
  ComponentProps<"input">,
  "type" | "children"
> & {
  /** The words beside the box; clicking them toggles it. */
  label: ReactNode;
  /** A muted line under the label, linked as the input's description. */
  description?: ReactNode;
  /** A bordered choice tile; native checkbox behaviour is unchanged. */
  tile?: boolean;
};

export function Checkbox({
  label,
  description,
  tile = false,
  className,
  style,
  id,
  "aria-describedby": describedBy,
  ...rest
}: CheckboxProps) {
  const fallbackId = useId();
  const descriptionId = description
    ? `${id ?? fallbackId}-description`
    : undefined;
  return (
    // `className` and `style` style the whole row (a toolbar's smaller, muted text; a bordered tile); the
    // box itself is fixed.
    <label
      className={[styles.checkbox, tile && styles.tile, className]
        .filter(Boolean)
        .join(" ")}
      style={style}
    >
      {/* type first, then the caller's props: they cannot turn this into another kind of input. */}
      <span className={styles.box}>
        <input
          type="checkbox"
          className={styles.input}
          id={id}
          {...rest}
          aria-describedby={
            [describedBy, descriptionId].filter(Boolean).join(" ") || undefined
          }
        />
        <Icon name="check" size={12} />
        <Icon name="minus" size={14} />
      </span>
      <span className={styles.text}>
        <span>{label}</span>
        {description ? (
          <span className={styles.description} id={descriptionId}>
            {description}
          </span>
        ) : null}
      </span>
    </label>
  );
}
