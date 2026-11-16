"use client";

import { useId } from "react";
import styles from "./brand-logo.module.css";

/** Small-size SVG contours keep diagonal cuts and letter gaps open at native size.
 * The horizontal layout, gradient, presets and nominal wordmark bounds are unchanged.
 * The wordmark uses the already bundled Pretendard instead of tiny traced letter outlines.
 * useId isolates gradient references when Docs renders several logos.
 */
export function BrandLogo({ size = "sidebar", variant = "full", decorative = false, className }: {
  size?: "sidebar" | "signIn"; variant?: "full" | "mark" | "compact"; decorative?: boolean; className?: string;
}) {
  const gradient = useId();
  const scale = (size === "signIn" ? 112.5 : 80) / 1428;
  // Tight artwork bounds keep a cropped mark centered without changing its contours.
  const width = variant === "compact" ? 248 : variant === "mark" ? 908 : 2417;
  return (
    <svg xmlns="http://www.w3.org/2000/svg" viewBox={`0 0 ${width} ${724 / 3}`}
      data-variant={variant}
      width={width * scale} height={724 / 3 * scale}
      className={[styles.logo, className].filter(Boolean).join(" ")}
      role={decorative ? undefined : "img"} aria-label={decorative ? undefined : "nginx-web-ui"}
      aria-hidden={decorative || undefined} focusable="false">
      <defs><linearGradient id={gradient} x1="286" y1="0" x2="1713" y2="0" gradientUnits="userSpaceOnUse">
        <stop offset="0" stopColor="#00a33a" /><stop offset="0.48" stopColor="#00896c" /><stop offset="1" stopColor="#00539a" />
      </linearGradient></defs>
      <path transform={`scale(${2 / 3}) translate(-316 -115)`}
        d="M426 326 366 383 317 334V475H451L411 431 468 370Z M316 163C316 136 333 117 357 117C373 117 386 122 397 128L618 375V314L653 276 687 314V434C687 458 669 477 647 477C638 477 630 477 624 476L604 466 385 220V284L357 314 316 265Z M688 117H546L588 161 535 216 578 263 639 204 688 253Z M728 115 808 115 866 344 931 115 1010 115 1075 352 1138 115 1219 115 1115 475 1031 475 970 238 903 475 823 475Z M1259 115H1341V326C1341 372 1365 400 1406 400S1474 375 1474 333V115H1556V335C1556 426 1501 477 1412 477S1259 430 1259 350Z M1621 115C1607 115 1596 126 1596 141V475H1678V115Z"
        fill={`url(#${gradient})`} fillRule="evenodd" />
      {/* Pretendard 600: measured ink height 201 and descender 44.3914 in source units.
          Keep the existing wordmark width/bottom while letting the font renderer draw curves. */}
      {variant === "full" ? <text className={styles.wordmark} x="1052" y="196.94196" fontSize="211.42681"
        textLength="1365" lengthAdjust="spacingAndGlyphs" fill="currentColor">
        nginx-web-ui
      </text> : null}
    </svg>
  );
}
