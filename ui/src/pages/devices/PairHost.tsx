import { Button } from "../../components/Button";
import { ErrorMessage } from "../../components/Feedback";
import { fmt, useI18n } from "../../i18n";
import { agentCall, Method, useAction, useNow } from "../../lib/agent";
import { countdown } from "../../lib/format";

interface PairHostProps {
  pin: string;
  expiresUnixMs: number;
  attemptsLeft: number;
  addresses: string[];
}

export function PairHost({ pin, expiresUnixMs, attemptsLeft, addresses }: PairHostProps) {
  const { t } = useI18n();
  const now = useNow(500);
  const cancel = useAction();
  const remaining = expiresUnixMs - now;

  return (
    <div className="grid justify-items-center gap-4 py-2 text-center">
      <p className="text-sm text-neutral-600 dark:text-neutral-400">{t.devices.hostWaiting}</p>
      <p
        className="selectable font-mono text-5xl font-semibold tracking-widest text-neutral-900 tabular-nums dark:text-neutral-50"
        aria-live="polite"
      >
        {pin.slice(0, 3)}
        <span className="inline-block w-4" aria-hidden="true" />
        {pin.slice(3)}
      </p>
      <p className="text-sm text-neutral-600 tabular-nums dark:text-neutral-400">
        {fmt(t.devices.hostExpires, { time: countdown(remaining) })}
        <span aria-hidden="true"> · </span>
        {fmt(t.devices.hostAttempts, { count: attemptsLeft })}
      </p>
      {addresses.length > 0 && (
        <div className="grid gap-1">
          <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.devices.hostAddress}</p>
          <p className="selectable font-mono text-sm text-neutral-800 dark:text-neutral-200">
            {addresses.join("  ·  ")}
          </p>
        </div>
      )}
      {cancel.error && <ErrorMessage error={cancel.error} onDismiss={cancel.clear} />}
      <Button busy={cancel.busy} onClick={() => void cancel.run(() => agentCall(Method.PairingCancel))}>
        {t.devices.hostCancel}
      </Button>
    </div>
  );
}
