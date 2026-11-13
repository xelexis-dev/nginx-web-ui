import type { AppShellNavGroup, RenderLink } from "./app-shell";
import type { IconName } from "./icon";
import { SectionNavScroll } from "./section-nav-scroll";
import styles from "./section-nav.module.css";
/** The item whose href is the longest match of the path ("/" and match:"exact" only match exactly). */
export function activeNavHref(
  path: string,
  nav: AppShellNavGroup[],
): string | null {
  let best: string | null = null;
  for (const group of nav) {
    for (const item of group.items) {
      const exact = item.match === "exact" || item.href === "/";
      const hit = exact
        ? path === item.href
        : path === item.href ||
          path.startsWith(
            item.href.endsWith("/") ? item.href : `${item.href}/`,
          );
      if (hit && (best === null || item.href.length > best.length))
        best = item.href;
    }
  }
  return best;
}

export interface SectionNavItem {
  href: string;
  label: string;
}
export interface SectionNavProps {
  items?: SectionNavItem[];
  groups?: {
    id: string;
    label: string;
    icon?: IconName;
    items: SectionNavItem[];
  }[];
  currentPath: string;
  variant?: "tabs";
  label: string;
  renderLink?: RenderLink;
}
export function SectionNav({
  items = [],
  groups,
  currentPath,
  label,
  renderLink,
}: SectionNavProps) {
  const sections: NonNullable<SectionNavProps["groups"]> = groups ?? [
    { id: "sections", label: "", items },
  ];
  const active = activeNavHref(currentPath, sections);
  const link: RenderLink = renderLink ?? ((props) => <a {...props} />);
  return (
    <SectionNavScroll active={active} className={styles.nav!} label={label}>
      {sections.map((group) => (
        <div className={styles.group} key={group.id}>
          <ul className={styles.list} aria-label={group.label || undefined}>
            {group.items.map((item) => (
              <li key={item.href}>
                {link({
                  href: item.href,
                  className: styles.item!,
                  "aria-current": active === item.href ? "page" : undefined,
                  children: item.label,
                })}
              </li>
            ))}
          </ul>
        </div>
      ))}
    </SectionNavScroll>
  );
}
