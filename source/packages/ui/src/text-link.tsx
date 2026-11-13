import type { ComponentPropsWithoutRef } from "react";
import styles from "./text-link.module.css";

export interface LinkStyleOptions {
  variant?: "inline" | "back";
  className?: string;
}
/** Use with router links to keep routing behavior and share the visual treatment. */
export function linkClassName({
  variant = "inline",
  className,
}: LinkStyleOptions = {}) {
  return [styles.link, variant === "back" ? styles.back : "", className]
    .filter(Boolean)
    .join(" ");
}
export type TextLinkProps = ComponentPropsWithoutRef<"a"> & LinkStyleOptions;
export function TextLink({ variant, className, ...props }: TextLinkProps) {
  return <a {...props} className={linkClassName({ variant, className })} />;
}
