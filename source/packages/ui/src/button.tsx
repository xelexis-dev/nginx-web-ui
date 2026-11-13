import type { ComponentProps } from "react";
import styles from "./button.module.css";

export type ButtonVariant =
  "primary" | "secondary" | "danger" | "ghost" | "dangerQuiet";
export type ButtonSize = "md" | "sm" | "lg" | "xs";

export interface ButtonProps extends ComponentProps<"button"> {
  /** 시각적 역할. 기본값 primary는 이전 Button과 같은 모습이다. */
  variant?: ButtonVariant;
  /** md 32px, sm 28px, lg 40px (auth), xs 24px. */
  size?: ButtonSize;
}

/**
 * 공통 버튼. hover·눌림·키보드 포커스·비활성 상태를 CSS로 가진다(이전에는 인라인 스타일뿐이라
 * 비활성 버튼이 활성과 똑같아 보였다). 기본 규칙은 모두 `:where()` 안에 있어 특이도가 0이므로,
 * 호출하는 쪽의 className이 항상 이기고 style prop은 그 위에서 이긴다. `type`은 지정하지 않으면
 * 브라우저 기본값(폼 안에서 submit)을 그대로 쓴다.
 */
export function Button({
  variant = "primary",
  size = "md",
  className,
  ...rest
}: ButtonProps) {
  return (
    <button
      {...rest}
      className={buttonClassName({ variant, size, className })}
    />
  );
}

export interface IconButtonProps extends Omit<
  ButtonProps,
  "aria-label" | "aria-labelledby"
> {
  /** 아이콘만 표시하므로 동작을 설명하는 접근성 이름이 필수다. */
  label: string;
}

/** Button의 상태·색상·포커스를 공유하는 정사각형 버튼. ref도 실제 button으로 전달한다. */
export function IconButton({
  label,
  size = "md",
  variant = size === "sm" ? "ghost" : "secondary",
  type = "button",
  className,
  ...rest
}: IconButtonProps) {
  return (
    <Button
      {...rest}
      size={size}
      type={type}
      variant={variant}
      aria-label={label}
      className={[styles.icon, className].filter(Boolean).join(" ")}
    />
  );
}

/**
 * The Button look for something that is not a <button>, e.g. a next/link that navigates ("새 서버",
 * "수정"). Same classes, so a link and a button of one variant look the same. The rules sit in
 * :where(), so an extra className still wins.
 */
export function buttonClassName({
  variant = "primary",
  size = "md",
  className,
}: {
  variant?: ButtonVariant;
  size?: ButtonSize;
  className?: string;
} = {}): string {
  return [
    styles.button,
    styles[variant],
    variant === "primary" ? "nwui-btn-primary" : null,
    size !== "md" ? styles[size] : null,
    className,
  ]
    .filter(Boolean)
    .join(" ");
}
