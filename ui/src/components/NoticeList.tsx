import { errorText, useI18n } from "../i18n";
import { agentCall, Method, useAction, useNow } from "../lib/agent";
import { dateTime, relativeTime } from "../lib/format";
import type { Notice, NoticeLevel } from "../lib/types";
import { IconButton } from "./Button";
import { ErrorMessage } from "./Feedback";
import { Icon, type IconName } from "./Icon";

const LEVEL: Record<NoticeLevel, { icon: IconName; className: string }> = {
  info: { icon: "info", className: "text-sky-600 dark:text-sky-400" },
  warning: { icon: "alert", className: "text-amber-600 dark:text-amber-400" },
  error: { icon: "alert", className: "text-red-600 dark:text-red-400" },
};

export function NoticeList({ notices }: { notices: Notice[] }) {
  const { t, locale } = useI18n();
  const now = useNow(30_000);
  const dismiss = useAction();

  return (
    <div className="grid gap-2">
      {dismiss.error && <ErrorMessage error={dismiss.error} onDismiss={dismiss.clear} />}
      <ul className="grid gap-2">
        {[...notices].reverse().map((notice) => {
          const level = LEVEL[notice.level];
          return (
            <li
              key={notice.id}
              className="flex items-start gap-3 rounded-lg bg-black/3 px-3.5 py-2.5 dark:bg-white/5"
            >
              <Icon name={level.icon} className={`mt-0.5 size-4 ${level.className}`} />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
                  {errorText(t, notice.code)}
                </p>
                <p className="selectable mt-0.5 break-words text-xs text-neutral-600 dark:text-neutral-400">
                  {notice.message}
                </p>
                <p
                  className="mt-1 text-xs text-neutral-400 dark:text-neutral-500"
                  title={dateTime(notice.at_unix_ms, locale)}
                >
                  {relativeTime(notice.at_unix_ms, now, locale)}
                </p>
              </div>
              <IconButton
                icon="x"
                label={t.common.dismiss}
                disabled={dismiss.busy}
                onClick={() => void dismiss.run(() => agentCall(Method.NoticeDismiss, { id: notice.id }))}
                className="-my-1 -mr-1.5"
              />
            </li>
          );
        })}
      </ul>
    </div>
  );
}
