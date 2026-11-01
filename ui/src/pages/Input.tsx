import { useCallback, useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import { AgentOfflineCard } from "../components/AgentOfflineCard";
import { Button } from "../components/Button";
import { Card } from "../components/Card";
import { ErrorMessage } from "../components/Feedback";
import { Field, inputClass, Toggle } from "../components/Field";
import { PageHeader } from "../components/PageHeader";
import { fmt, useI18n } from "../i18n";
import { agentCall, Method, useAction, useAgent } from "../lib/agent";
import { useConfigSaver } from "../lib/config";
import { hotkeyFromEvent, MAX_EDGE_DELAY_MS, parseEdgeDelay, sameHotkey } from "../lib/hotkey";
import type { Config, InputConfig, KvmStatus, ServiceStatus } from "../lib/types";

type HotkeyField = "hotkey_switch_local" | "hotkey_switch_remote" | "hotkey_lock" | "hotkey_gaming";

const HOTKEY_FIELDS: HotkeyField[] = [
  "hotkey_switch_local",
  "hotkey_switch_remote",
  "hotkey_lock",
  "hotkey_gaming",
];

const DEFAULT_HOTKEYS: Record<HotkeyField, string> = {
  hotkey_switch_local: "Ctrl+Alt+F1",
  hotkey_switch_remote: "Ctrl+Alt+F2",
  hotkey_lock: "ScrollLock",
  hotkey_gaming: "Ctrl+Alt+G",
};

const isDefault = (field: HotkeyField, value: string) =>
  value === DEFAULT_HOTKEYS[field] || (value !== "" && sameHotkey(value, DEFAULT_HOTKEYS[field]));

export function Input() {
  const { t } = useI18n();
  const snapshot = useAgent();
  const status = snapshot.connected ? snapshot.status : null;
  const config = snapshot.config;

  return (
    <>
      <PageHeader title={t.nav.input} />
      {status && config ? (
        <div className="grid gap-4">
          <SwitchingCard config={config} kvm={status.kvm} />
          <ServiceCard />
          <HotkeysCard config={config} />
        </div>
      ) : (
        <AgentOfflineCard error={snapshot.error} />
      )}
    </>
  );
}

function hostName(kvm: KvmStatus): string | null {
  const layout = kvm.layout;
  if (!layout) return null;
  return layout.devices.find((d) => d.device_id === layout.input_host)?.device_name ?? null;
}

function SwitchingCard({ config, kvm }: { config: Config; kvm: KvmStatus }) {
  const { t } = useI18n();
  const saver = useConfigSaver();
  const toggle = useAction();
  const host = hostName(kvm);

  const setToggle = (method: typeof Method.KvmSetLock | typeof Method.KvmSetGaming, enabled: boolean) =>
    void toggle.run(() => agentCall(method, { enabled }));

  return (
    <Card
      title={t.input.switchingTitle}
      description={host && kvm.role === "remote" ? fmt(t.input.hostOnly, { name: host }) : undefined}
    >
      <div className="grid gap-5">
        <EdgeDelayField
          value={config.input.edge_delay_ms}
          busy={saver.busy}
          onSave={(edge_delay_ms) => saver.save((c) => ({ ...c, input: { ...c.input, edge_delay_ms } }))}
        />
        <div className="grid gap-4 border-t border-black/5 pt-4 dark:border-white/10">
          <Toggle
            label={t.input.lock}
            hint={t.input.lockHint}
            checked={kvm.locked}
            disabled={toggle.busy}
            onChange={(on) => setToggle(Method.KvmSetLock, on)}
          />
          <Toggle
            label={t.input.gaming}
            hint={t.input.gamingHint}
            checked={kvm.gaming}
            disabled={toggle.busy}
            onChange={(on) => setToggle(Method.KvmSetGaming, on)}
          />
          <Toggle
            label={t.input.gamingAuto}
            hint={t.input.gamingAutoHint}
            checked={config.input.gaming_auto}
            disabled={saver.busy}
            onChange={(on) => void saver.save((c) => ({ ...c, input: { ...c.input, gaming_auto: on } }))}
          />
          <p className="rounded-md bg-amber-50 px-3 py-2 text-xs text-amber-800 ring-1 ring-amber-200 dark:bg-amber-500/10 dark:text-amber-200 dark:ring-amber-500/20">
            {t.input.gamingWarning}
          </p>
        </div>
        {saver.error && <ErrorMessage error={saver.error} onDismiss={saver.clear} />}
        {toggle.error && <ErrorMessage error={toggle.error} onDismiss={toggle.clear} />}
      </div>
    </Card>
  );
}

function ServiceCard() {
  const { t } = useI18n();
  const action = useAction();
  const [svc, setSvc] = useState<ServiceStatus | null>(null);

  const refresh = useCallback(async () => {
    try {
      setSvc(await agentCall<ServiceStatus>(Method.ServiceStatus));
    } catch {
      setSvc(null);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = (method: typeof Method.ServiceInstall | typeof Method.ServiceUninstall) =>
    void action.run(async () => setSvc(await agentCall<ServiceStatus>(method)));

  const state = svc?.installed
    ? svc.running
      ? { label: t.input.serviceStateInstalled, tone: "ok" as const }
      : { label: t.input.serviceStateStopped, tone: "warn" as const }
    : { label: t.input.serviceStateMissing, tone: "off" as const };

  const dot =
    state.tone === "ok"
      ? "bg-emerald-500"
      : state.tone === "warn"
        ? "bg-amber-500"
        : "bg-neutral-400 dark:bg-neutral-500";

  return (
    <Card
      title={t.input.serviceTitle}
      description={t.input.serviceIntro}
      actions={
        svc?.installed ? (
          <Button variant="danger" busy={action.busy} onClick={() => run(Method.ServiceUninstall)}>
            {t.input.serviceUninstall}
          </Button>
        ) : (
          <Button variant="primary" busy={action.busy} onClick={() => run(Method.ServiceInstall)}>
            {t.input.serviceInstall}
          </Button>
        )
      }
    >
      <div className="grid gap-3">
        <div className="flex items-center gap-2 text-sm text-neutral-700 dark:text-neutral-300">
          <span className={`size-2 rounded-full ${dot}`} aria-hidden />
          {state.label}
        </div>
        {!svc?.installed && (
          <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.input.serviceAdminNote}</p>
        )}
        <p className="rounded-md bg-amber-50 px-3 py-2 text-xs text-amber-800 ring-1 ring-amber-200 dark:bg-amber-500/10 dark:text-amber-200 dark:ring-amber-500/20">
          {t.input.serviceAnticheat}
        </p>
        {action.error && <ErrorMessage error={action.error} onDismiss={action.clear} />}
      </div>
    </Card>
  );
}

interface EdgeDelayFieldProps {
  value: number;
  busy: boolean;
  onSave: (value: number) => Promise<boolean>;
}

function EdgeDelayField({ value, busy, onSave }: EdgeDelayFieldProps) {
  const { t } = useI18n();
  const [draft, setDraft] = useState(String(value));

  useEffect(() => setDraft(String(value)), [value]);

  const parsed = parseEdgeDelay(draft);
  const dirty = draft.trim() !== String(value);
  const invalid = dirty && parsed === null;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!dirty || parsed === null) return;
    void onSave(parsed);
  };

  return (
    <form onSubmit={submit}>
      <Field label={t.input.edgeDelay} hint={t.input.edgeDelayHint} error={invalid ? t.input.edgeDelayInvalid : null}>
        {(id, describedBy) => (
          <div className="flex gap-2">
            <input
              id={id}
              value={draft}
              inputMode="numeric"
              maxLength={String(MAX_EDGE_DELAY_MS).length}
              aria-describedby={describedBy}
              aria-invalid={invalid || undefined}
              onChange={(e) => setDraft(e.currentTarget.value)}
              className={`${inputClass} w-28 font-mono tabular-nums`}
            />
            {dirty && (
              <Button type="submit" variant="primary" busy={busy} disabled={parsed === null}>
                {t.common.save}
              </Button>
            )}
          </div>
        )}
      </Field>
    </form>
  );
}

function HotkeysCard({ config }: { config: Config }) {
  const { t } = useI18n();
  const saver = useConfigSaver();
  const [recording, setRecording] = useState<HotkeyField | null>(null);
  const [conflict, setConflict] = useState<string | null>(null);

  const apply = (field: HotkeyField, value: string) => {
    setRecording(null);
    if (value) {
      const clash = HOTKEY_FIELDS.find((f) => f !== field && sameHotkey(config.input[f], value));
      if (clash) {
        setConflict(fmt(t.input.hotkeyDuplicate, { key: value, action: t.input.hotkeys[clash] }));
        return;
      }
    }
    setConflict(null);
    if (config.input[field] === value) return;
    void saver.save((c) => ({ ...c, input: { ...c.input, [field]: value } as InputConfig }));
  };

  const changed = HOTKEY_FIELDS.some((f) => !isDefault(f, config.input[f]));
  const resetAll = () => {
    setRecording(null);
    setConflict(null);
    void saver.save((c) => ({ ...c, input: { ...c.input, ...DEFAULT_HOTKEYS } }));
  };

  return (
    <Card
      title={t.input.hotkeysTitle}
      description={t.input.hotkeysIntro}
      actions={
        changed && (
          <Button variant="secondary" icon="refresh" disabled={saver.busy} onClick={resetAll}>
            {t.input.hotkeyResetAll}
          </Button>
        )
      }
    >
      <ul className="grid divide-y divide-black/5 dark:divide-white/10">
        {HOTKEY_FIELDS.map((field) => (
          <li key={field} className="flex flex-wrap items-center justify-between gap-3 py-3 first:pt-0 last:pb-0">
            <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">{t.input.hotkeys[field]}</span>
            <HotkeyRecorder
              value={config.input[field]}
              label={t.input.hotkeys[field]}
              recording={recording === field}
              busy={saver.busy}
              onStart={() => {
                setConflict(null);
                setRecording(field);
              }}
              onCancel={() => setRecording(null)}
              onCapture={(value) => apply(field, value)}
              onReset={isDefault(field, config.input[field]) ? undefined : () => apply(field, DEFAULT_HOTKEYS[field])}
              defaultValue={DEFAULT_HOTKEYS[field]}
            />
          </li>
        ))}
      </ul>
      {(conflict || saver.error) && (
        <div className="mt-4 grid gap-2">
          {conflict && <ErrorMessage error={{ code: "config.invalid", message: conflict }} onDismiss={() => setConflict(null)} />}
          {saver.error && <ErrorMessage error={saver.error} onDismiss={saver.clear} />}
        </div>
      )}
    </Card>
  );
}

interface HotkeyRecorderProps {
  value: string;
  label: string;
  recording: boolean;
  busy: boolean;
  onStart: () => void;
  onCancel: () => void;
  onCapture: (value: string) => void;
  onReset?: () => void;
  defaultValue: string;
}

function HotkeyRecorder({ value, label, recording, busy, onStart, onCancel, onCapture, onReset, defaultValue }: HotkeyRecorderProps) {
  const { t } = useI18n();
  const ref = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (recording) ref.current?.focus();
  }, [recording]);

  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
      onCancel();
      return;
    }
    const hotkey = hotkeyFromEvent(e.nativeEvent);
    if (hotkey) onCapture(hotkey);
  };

  return (
    <div className="flex items-center gap-2">
      {recording ? (
        <button
          ref={ref}
          type="button"
          aria-label={`${label}: ${t.input.hotkeyRecording}`}
          onKeyDown={onKeyDown}
          onBlur={onCancel}
          className="min-w-44 animate-pulse rounded-md bg-indigo-50 px-3 py-1.5 text-center text-sm text-indigo-900 ring-2 ring-indigo-500 focus:outline-none dark:bg-indigo-500/15 dark:text-indigo-100"
        >
          {t.input.hotkeyRecording}
        </button>
      ) : (
        <kbd
          className={`min-w-28 rounded-md px-3 py-1.5 text-center font-mono text-sm ring-1 ${
            value
              ? "bg-white/90 text-neutral-900 ring-black/10 dark:bg-black/20 dark:text-neutral-100 dark:ring-white/15"
              : "text-neutral-500 ring-black/10 dark:text-neutral-400 dark:ring-white/15"
          }`}
        >
          {value || t.input.hotkeyOff}
        </kbd>
      )}
      {!recording && (
        <>
          <Button variant="secondary" disabled={busy} onClick={onStart}>
            {t.input.hotkeyChange}
          </Button>
          {value && (
            <Button variant="ghost" disabled={busy} onClick={() => onCapture("")}>
              {t.input.hotkeyDisable}
            </Button>
          )}
          {onReset && (
            <Button
              variant="ghost"
              icon="refresh"
              disabled={busy}
              title={fmt(t.input.hotkeyResetTo, { key: defaultValue })}
              onClick={onReset}
            >
              {t.input.hotkeyReset}
            </Button>
          )}
        </>
      )}
    </div>
  );
}
