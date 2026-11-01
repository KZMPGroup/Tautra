import type { ButtonHTMLAttributes, ReactNode } from "react";
import { Icon, Spinner, type IconName } from "./Icon";

type Variant = "primary" | "secondary" | "danger" | "ghost";

const VARIANTS: Record<Variant, string> = {
  primary:
    "bg-indigo-600 text-white hover:bg-indigo-500 active:bg-indigo-700 dark:bg-indigo-500 dark:hover:bg-indigo-400",
  secondary:
    "bg-white/80 text-neutral-800 ring-1 ring-black/10 hover:bg-white active:bg-neutral-100 dark:bg-white/10 dark:text-neutral-100 dark:ring-white/10 dark:hover:bg-white/15",
  danger:
    "bg-red-600 text-white hover:bg-red-500 active:bg-red-700 dark:bg-red-500 dark:hover:bg-red-400",
  ghost:
    "text-neutral-700 hover:bg-black/5 active:bg-black/10 dark:text-neutral-200 dark:hover:bg-white/10",
};

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  icon?: IconName;
  busy?: boolean;
  children?: ReactNode;
}

export function Button({
  variant = "secondary",
  icon,
  busy = false,
  disabled,
  className = "",
  children,
  type = "button",
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      disabled={disabled || busy}
      aria-busy={busy || undefined}
      className={`inline-flex items-center justify-center gap-2 rounded-md px-3.5 py-1.5 text-sm font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-500 disabled:cursor-not-allowed disabled:opacity-50 ${VARIANTS[variant]} ${className}`}
      {...rest}
    >
      {busy ? <Spinner /> : icon ? <Icon name={icon} className="size-4" /> : null}
      {children}
    </button>
  );
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  icon: IconName;
  label: string;
}

export function IconButton({ icon, label, className = "", type = "button", ...rest }: IconButtonProps) {
  return (
    <button
      type={type}
      title={label}
      aria-label={label}
      className={`inline-flex size-8 items-center justify-center rounded-md text-neutral-500 transition-colors hover:bg-black/5 hover:text-neutral-800 focus-visible:outline-2 focus-visible:outline-indigo-500 dark:text-neutral-400 dark:hover:bg-white/10 dark:hover:text-neutral-100 ${className}`}
      {...rest}
    >
      <Icon name={icon} className="size-4" />
    </button>
  );
}
