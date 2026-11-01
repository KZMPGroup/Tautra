import type { ReactNode } from "react";

interface CardProps {
  title?: string;
  description?: ReactNode;
  actions?: ReactNode;
  className?: string;
  children?: ReactNode;
}

export function Card({ title, description, actions, className = "", children }: CardProps) {
  return (
    <section
      className={`rounded-xl bg-white/75 p-5 ring-1 ring-black/5 shadow-sm dark:bg-white/4 dark:ring-white/10 ${className}`}
    >
      {(title || actions) && (
        <header className="mb-4 flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            {title && (
              <h2 className="text-base font-semibold text-neutral-900 dark:text-neutral-50">{title}</h2>
            )}
            {description && (
              <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">{description}</p>
            )}
          </div>
          {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
        </header>
      )}
      {children}
    </section>
  );
}

export function Detail({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-xs text-neutral-500 dark:text-neutral-400">{label}</dt>
      <dd className="selectable mt-0.5 truncate text-sm font-medium text-neutral-900 dark:text-neutral-100">
        {children}
      </dd>
    </div>
  );
}
