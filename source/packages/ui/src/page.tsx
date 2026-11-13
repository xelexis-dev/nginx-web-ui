import type { ReactNode } from "react";
import { InfoTip } from "./info-tip";
import styles from "./page.module.css";

/** Server-safe page structure: the page header, a titled surface and an empty state (DESIGN.md). */

export interface PageHeaderProps {
  /** Above the title: a back link or a crumb (pass your own <Link>). */
  eyebrow?: ReactNode;
  title: ReactNode;
  description?: ReactNode;
  info?: ReactNode;
  /** Buttons or button-styled links; they wrap under the title on narrow screens. */
  actions?: ReactNode;
}

export function PageHeader({
  eyebrow,
  title,
  description,
  info,
  actions,
}: PageHeaderProps) {
  return (
    <header className={styles.pageHeader}>
      <div className={styles.heading}>
        {eyebrow ? <div className={styles.eyebrow}>{eyebrow}</div> : null}
        <div className={styles.titleRow}>
          <h1 className={styles.title}>{title}</h1>
          {info ? <InfoTip label="페이지 설명">{info}</InfoTip> : null}
        </div>
        {description ? (
          <div className={styles.description}>{description}</div>
        ) : null}
      </div>
      {actions ? <div className={styles.actions}>{actions}</div> : null}
    </header>
  );
}

export interface SurfaceProps {
  /** Layout-only placement in a parent grid; visual styling remains shared. */
  className?: string;
  id?: string;
  "aria-labelledby"?: string;
  title?: ReactNode;
  description?: ReactNode;
  info?: ReactNode;
  actions?: ReactNode;
  /** Search, filters and counts above the surface body. */
  toolbar?: ReactNode;
  /** No body padding and a rule under the header: for a table or list that runs to the edges. */
  flush?: boolean;
  as?: "section" | "div";
  headingLevel?: 2 | 3;
  children: ReactNode;
}

export function Surface({
  className,
  id,
  "aria-labelledby": labelledBy,
  title,
  description,
  info,
  actions,
  toolbar,
  flush = false,
  as: Tag = "section",
  headingLevel = 2,
  children,
}: SurfaceProps) {
  const Heading = headingLevel === 3 ? "h3" : "h2";
  const hasHeader = title || description || actions;
  return (
    <Tag
      id={id}
      aria-labelledby={labelledBy}
      className={[styles.surface, className].filter(Boolean).join(" ")}
      data-flush={flush ? "" : undefined}
    >
      {hasHeader ? (
        <div className={styles.surfaceHeader}>
          <div className={styles.surfaceHeading}>
            {title ? (
              <div className={styles.titleRow}>
                <Heading className={styles.surfaceTitle}>{title}</Heading>
                {info ? (
                  <InfoTip
                    label={typeof title === "string" ? `${title} 설명` : "설명"}
                  >
                    {info}
                  </InfoTip>
                ) : null}
              </div>
            ) : null}
            {description ? (
              <p className={styles.surfaceDescription}>{description}</p>
            ) : null}
          </div>
          {actions ? (
            <div className={styles.surfaceActions}>{actions}</div>
          ) : null}
        </div>
      ) : null}
      {toolbar ? <div className={styles.toolbar}>{toolbar}</div> : null}
      <div className={styles.surfaceBody}>{children}</div>
    </Tag>
  );
}

export interface EmptyStateProps {
  title: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
}

export function EmptyState({ title, description, action }: EmptyStateProps) {
  return (
    <div className={styles.empty}>
      <p className={styles.emptyTitle}>{title}</p>
      {description ? <p className={styles.emptyText}>{description}</p> : null}
      {action ? <div className={styles.emptyAction}>{action}</div> : null}
    </div>
  );
}
