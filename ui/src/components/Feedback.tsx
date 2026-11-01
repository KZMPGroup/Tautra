import { describeError, fmt, useI18n } from "../i18n";
import type { UiError } from "../lib/types";
import { IconButton } from "./Button";
import { Icon } from "./Icon";

interface ErrorMessageProps {
  error: UiError;
  onDismiss?: () => void;
}

export function ErrorMessage({ error, onDismiss }: ErrorMessageProps) {
  const { t } = useI18n();
  const { title, detail } = describeError(t, error);
  return (
    <div
      role="alert"
      className="flex items-start gap-3 rounded-lg bg-red-50 px-3.5 py-2.5 text-sm text-red-900 ring-1 ring-red-200 dark:bg-red-500/10 dark:text-red-200 dark:ring-red-500/30"
    >
      <Icon name="alert" className="mt-0.5 size-4 text-red-600 dark:text-red-400" />
      <div className="min-w-0 flex-1">
        <p className="font-medium">{title}</p>
        {detail && detail !== title && (
          <p className="selectable mt-0.5 break-words text-xs opacity-80">
            {fmt(t.common.detail, { message: detail })}
          </p>
        )}
      </div>
      {onDismiss && <IconButton icon="x" label={t.common.dismiss} onClick={onDismiss} className="-my-1 -me-1.5" />}
    </div>
  );
}

export function SuccessMessage({ text }: { text: string }) {
  return (
    <div
      role="status"
      className="flex items-center gap-2 rounded-lg bg-emerald-50 px-3.5 py-2.5 text-sm text-emerald-900 ring-1 ring-emerald-200 dark:bg-emerald-500/10 dark:text-emerald-200 dark:ring-emerald-500/30"
    >
      <Icon name="check" className="size-4 text-emerald-600 dark:text-emerald-400" />
      <span>{text}</span>
    </div>
  );
}
