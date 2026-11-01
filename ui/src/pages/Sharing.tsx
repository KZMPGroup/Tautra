import { useEffect, useState, type FormEvent } from "react";
import { AgentOfflineCard } from "../components/AgentOfflineCard";
import { Button, IconButton } from "../components/Button";
import { Card } from "../components/Card";
import { ErrorMessage } from "../components/Feedback";
import { Field, inputClass, Toggle } from "../components/Field";
import { Icon } from "../components/Icon";
import { PageHeader } from "../components/PageHeader";
import { fmt, useI18n } from "../i18n";
import type { Strings } from "../i18n/strings";
import { agentCall, Method, useAction, useAgent, useNow } from "../lib/agent";
import { useConfigSaver } from "../lib/config";
import { bytes, relativeTime } from "../lib/format";
import type { ClipActivity, ShareConfig, ShareStatus, TransferState, TransferView } from "../lib/types";

type FormatKey = "text" | "html" | "rtf" | "image" | "files";

const FORMATS: FormatKey[] = ["text", "html", "rtf", "image", "files"];

const MIN_CLIPBOARD_MB = 1;
const MAX_CLIPBOARD_MB = 1024;

const ACTIVE_STATES: TransferState[] = ["running", "waiting"];

const STATE_BADGE: Record<TransferState, string> = {
  running: "bg-indigo-100 text-indigo-800 dark:bg-indigo-500/15 dark:text-indigo-300",
  waiting: "bg-amber-100 text-amber-800 dark:bg-amber-500/15 dark:text-amber-300",
  done: "bg-emerald-100 text-emerald-800 dark:bg-emerald-500/15 dark:text-emerald-300",
  failed: "bg-red-100 text-red-800 dark:bg-red-500/15 dark:text-red-300",
  cancelled: "bg-neutral-200 text-neutral-700 dark:bg-white/10 dark:text-neutral-300",
};

export function Sharing() {
  const { t } = useI18n();
  const snapshot = useAgent();
  const status = snapshot.connected ? snapshot.status : null;
  const config = snapshot.config;

  return (
    <>
      <PageHeader title={t.nav.sharing} />
      {status && config ? (
        <div className="grid gap-4">
          <ClipboardCard config={config.share} share={status.share} />
          <FilesCard config={config.share} share={status.share} />
          <TransfersCard transfers={status.share.transfers} />
        </div>
      ) : (
        <AgentOfflineCard error={snapshot.error} />
      )}
    </>
  );
}

function useShareSaver() {
  const saver = useConfigSaver();
  const update = (patch: Partial<ShareConfig>) =>
    saver.save((c) => ({ ...c, share: { ...c.share, ...patch } }));
  return { ...saver, update };
}

function WarningNote({ text }: { text: string }) {
  return (
    <p className="flex items-start gap-2 rounded-lg bg-amber-50 px-3.5 py-2.5 text-sm text-amber-900 ring-1 ring-amber-200 dark:bg-amber-500/10 dark:text-amber-200 dark:ring-amber-500/30">
      <Icon name="alert" className="mt-0.5 size-4 text-amber-600 dark:text-amber-400" />
      <span>{text}</span>
    </p>
  );
}

function ClipboardCard({ config, share }: { config: ShareConfig; share: ShareStatus }) {
  const { t } = useI18n();
  const saver = useShareSaver();

  return (
    <Card title={t.sharing.clipboardTitle} description={t.sharing.clipboardIntro}>
      <div className="grid gap-5">
        <Toggle
          label={t.sharing.clipboard}
          hint={t.sharing.clipboardHint}
          checked={config.clipboard}
          disabled={saver.busy}
          onChange={(clipboard) => void saver.update({ clipboard })}
        />
        {config.clipboard && !share.clipboard_ready && <WarningNote text={t.sharing.clipboardNotReady} />}
        <fieldset className="grid gap-4 border-t border-black/5 pt-4 dark:border-white/10" disabled={!config.clipboard}>
          <legend className="sr-only">{t.sharing.formatsTitle}</legend>
          <p aria-hidden="true" className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
            {t.sharing.formatsTitle}
          </p>
          {FORMATS.map((key) => (
            <Toggle
              key={key}
              label={t.sharing.formats[key]}
              hint={t.sharing.formatsHint[key]}
              checked={config[key]}
              disabled={saver.busy || !config.clipboard}
              onChange={(on) => void saver.update({ [key]: on })}
            />
          ))}
        </fieldset>
        <div className="border-t border-black/5 pt-4 dark:border-white/10">
          <MaxSizeField
            value={config.max_clipboard_mb}
            busy={saver.busy}
            disabled={!config.clipboard}
            onSave={(max_clipboard_mb) => saver.update({ max_clipboard_mb })}
          />
        </div>
        <ActivityList sent={share.last_sent} received={share.last_received} />
        {saver.error && <ErrorMessage error={saver.error} onDismiss={saver.clear} />}
      </div>
    </Card>
  );
}

function parseMaxSize(raw: string): number | null {
  const text = raw.trim();
  if (!/^\d{1,4}$/.test(text)) return null;
  const value = Number(text);
  return value >= MIN_CLIPBOARD_MB && value <= MAX_CLIPBOARD_MB ? value : null;
}

interface MaxSizeFieldProps {
  value: number;
  busy: boolean;
  disabled: boolean;
  onSave: (value: number) => Promise<boolean>;
}

function MaxSizeField({ value, busy, disabled, onSave }: MaxSizeFieldProps) {
  const { t } = useI18n();
  const [draft, setDraft] = useState(String(value));

  useEffect(() => setDraft(String(value)), [value]);

  const parsed = parseMaxSize(draft);
  const dirty = draft.trim() !== String(value);
  const invalid = dirty && parsed === null;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!dirty || parsed === null) return;
    void onSave(parsed);
  };

  return (
    <form onSubmit={submit}>
      <Field label={t.sharing.maxSize} hint={t.sharing.maxSizeHint} error={invalid ? t.sharing.maxSizeInvalid : null}>
        {(id, describedBy) => (
          <div className="flex gap-2">
            <input
              id={id}
              value={draft}
              inputMode="numeric"
              maxLength={String(MAX_CLIPBOARD_MB).length}
              disabled={disabled}
              aria-describedby={describedBy}
              aria-invalid={invalid || undefined}
              onChange={(e) => setDraft(e.currentTarget.value)}
              className={`${inputClass} w-28 font-mono tabular-nums`}
            />
            {dirty && (
              <Button type="submit" variant="primary" busy={busy} disabled={parsed === null || disabled}>
                {t.common.save}
              </Button>
            )}
          </div>
        )}
      </Field>
    </form>
  );
}

function activityText(t: Strings, activity: ClipActivity, now: number, locale: string): string {
  const kinds = activity.kinds
    .map((k) => (k === "files" && activity.files > 0 ? fmt(t.sharing.activityFiles, { count: activity.files }) : t.sharing.kinds[k]))
    .join(", ");
  return fmt(t.sharing.activity, {
    kinds,
    size: bytes(activity.bytes, locale),
    time: relativeTime(activity.at_unix_ms, now, locale),
  });
}

function ActivityList({ sent, received }: { sent: ClipActivity | null; received: ClipActivity | null }) {
  const { t, locale } = useI18n();
  const now = useNow(5000);
  const rows: [string, ClipActivity | null][] = [
    [t.sharing.lastSent, sent],
    [t.sharing.lastReceived, received],
  ];

  return (
    <dl className="grid gap-2 border-t border-black/5 pt-4 text-sm sm:grid-cols-[max-content_1fr] sm:gap-x-6 dark:border-white/10">
      {rows.map(([label, activity]) => (
        <div key={label} className="contents">
          <dt className="text-neutral-500 dark:text-neutral-400">{label}</dt>
          <dd className="min-w-0 text-neutral-900 dark:text-neutral-100">
            {activity ? activityText(t, activity, now, locale) : t.sharing.never}
          </dd>
        </div>
      ))}
    </dl>
  );
}

function FilesCard({ config, share }: { config: ShareConfig; share: ShareStatus }) {
  const { t } = useI18n();
  const saver = useShareSaver();

  return (
    <Card title={t.sharing.filesTitle}>
      <div className="grid gap-5">
        <Toggle
          label={t.sharing.dragDrop}
          hint={t.sharing.dragDropHint}
          checked={config.drag_drop}
          disabled={saver.busy}
          onChange={(drag_drop) => void saver.update({ drag_drop })}
        />
        {config.drag_drop && !share.drag_ready && <WarningNote text={t.sharing.dragNotReady} />}
        {saver.error && <ErrorMessage error={saver.error} onDismiss={saver.clear} />}
      </div>
    </Card>
  );
}

function TransfersCard({ transfers }: { transfers: TransferView[] }) {
  const { t } = useI18n();
  const clear = useAction();
  const finished = transfers.some((x) => !ACTIVE_STATES.includes(x.state));
  const ordered = [...transfers].sort((a, b) => b.started_unix_ms - a.started_unix_ms);

  return (
    <Card
      title={t.sharing.transfersTitle}
      actions={
        finished ? (
          <Button
            variant="ghost"
            icon="trash"
            busy={clear.busy}
            onClick={() => void clear.run(() => agentCall(Method.TransferClear))}
          >
            {t.sharing.clearFinished}
          </Button>
        ) : undefined
      }
    >
      {ordered.length === 0 ? (
        <p className="text-sm text-neutral-500 dark:text-neutral-400">{t.sharing.transfersEmpty}</p>
      ) : (
        <ul className="grid divide-y divide-black/5 dark:divide-white/10">
          {ordered.map((transfer) => (
            <TransferRow key={transfer.id} transfer={transfer} />
          ))}
        </ul>
      )}
      {clear.error && (
        <div className="mt-4">
          <ErrorMessage error={clear.error} onDismiss={clear.clear} />
        </div>
      )}
    </Card>
  );
}

function TransferRow({ transfer }: { transfer: TransferView }) {
  const { t, locale } = useI18n();
  const action = useAction();
  const now = useNow(5000);
  const active = ACTIVE_STATES.includes(transfer.state);
  const total = Math.max(transfer.total_bytes, 0);
  const done = Math.min(transfer.done_bytes, total);
  const percent = total > 0 ? Math.floor((done / total) * 100) : transfer.state === "done" ? 100 : 0;
  const incoming = transfer.direction === "incoming";
  const peer = fmt(incoming ? t.sharing.incoming : t.sharing.outgoing, { name: transfer.peer_name });
  const summary = fmt(t.sharing.itemsSummary, { files: transfer.files, size: bytes(total, locale) });
  const when = relativeTime(transfer.finished_unix_ms ?? transfer.started_unix_ms, now, locale);

  return (
    <li className="grid gap-2 py-3 first:pt-0 last:pb-0">
      <div className="flex items-start gap-3">
        <Icon
          name={incoming ? "download" : "upload"}
          className="mt-0.5 size-5 text-neutral-500 dark:text-neutral-400"
        />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium text-neutral-900 dark:text-neutral-100" title={transfer.name}>
            {transfer.name}
          </p>
          <p className="truncate text-xs text-neutral-500 dark:text-neutral-400">
            {peer} · {summary} · {when}
          </p>
        </div>
        <span className={`shrink-0 rounded-full px-2.5 py-0.5 text-xs font-medium ${STATE_BADGE[transfer.state]}`}>
          {t.sharing.state[transfer.state]}
        </span>
        <div className="flex shrink-0 items-center">
          {transfer.path && (
            <IconButton
              icon="folder"
              label={t.sharing.revealTransfer}
              disabled={action.busy}
              onClick={() => void action.run(() => agentCall(Method.TransferReveal, { id: transfer.id }))}
            />
          )}
          {active && (
            <IconButton
              icon="x"
              label={t.sharing.cancelTransfer}
              disabled={action.busy}
              onClick={() => void action.run(() => agentCall(Method.TransferCancel, { id: transfer.id }))}
            />
          )}
        </div>
      </div>
      {active && (
        <div className="grid gap-1 pl-8">
          <div
            role="progressbar"
            aria-label={transfer.name}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent}
            className="h-1.5 overflow-hidden rounded-full bg-black/10 dark:bg-white/10"
          >
            <div
              className={`h-full rounded-full transition-[width] duration-300 ${
                transfer.state === "waiting" ? "bg-amber-500" : "bg-indigo-600 dark:bg-indigo-400"
              }`}
              style={{ width: `${percent}%` }}
            />
          </div>
          <p className="text-xs tabular-nums text-neutral-500 dark:text-neutral-400">
            {fmt(t.sharing.progress, { done: bytes(done, locale), total: bytes(total, locale) })} · {percent}%
          </p>
        </div>
      )}
      {transfer.state === "failed" && transfer.error && (
        <p className="selectable pl-8 text-xs break-words text-red-700 dark:text-red-400">{transfer.error}</p>
      )}
      {action.error && (
        <div className="pl-8">
          <ErrorMessage error={action.error} onDismiss={action.clear} />
        </div>
      )}
    </li>
  );
}
