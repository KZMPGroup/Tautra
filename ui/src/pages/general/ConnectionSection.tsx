import { useEffect, useState, type FormEvent } from "react";
import { Button, IconButton } from "../../components/Button";
import { Card } from "../../components/Card";
import { ErrorMessage } from "../../components/Feedback";
import { Field, inputClass, Toggle } from "../../components/Field";
import { Icon } from "../../components/Icon";
import { fmt, useI18n } from "../../i18n";
import { agentCall, Method } from "../../lib/agent";
import { parsePort, useConfigSaver } from "../../lib/config";
import type { Config, NetworkConfig, NetworkStatus } from "../../lib/types";

interface ConnectionSectionProps {
  config: Config;
  network: NetworkStatus;
}

export function ConnectionSection({ config, network }: ConnectionSectionProps) {
  const { t } = useI18n();
  const saver = useConfigSaver();

  const saveNetwork = (patch: (n: NetworkConfig) => NetworkConfig) =>
    saver.save((c) => ({ ...c, network: patch(c.network) }));

  return (
    <Card title={t.general.connectionSection}>
      <div className="grid gap-5">
        <Toggle
          label={t.general.autoConnect}
          hint={t.general.autoConnectHint}
          checked={config.network.auto_connect}
          disabled={saver.busy}
          onChange={(auto_connect) => void saveNetwork((n) => ({ ...n, auto_connect }))}
        />

        <div className="border-t border-black/5 pt-4 dark:border-white/10">
          <AllowedNetworks
            config={config.network}
            network={network}
            busy={saver.busy}
            onSave={(patch) => void saveNetwork(patch)}
          />
        </div>

        <div className="border-t border-black/5 pt-4 dark:border-white/10">
          <PortField port={config.network.port} busy={saver.busy} onSave={(port) => saveNetwork((n) => ({ ...n, port }))} />
        </div>

        {saver.error && <ErrorMessage error={saver.error} onDismiss={saver.clear} />}
      </div>
    </Card>
  );
}

const sameName = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

interface AllowedNetworksProps {
  config: NetworkConfig;
  network: NetworkStatus;
  busy: boolean;
  onSave: (patch: (n: NetworkConfig) => NetworkConfig) => void;
}

function AllowedNetworks({ config, network, busy, onSave }: AllowedNetworksProps) {
  const { t } = useI18n();
  const restrict = config.restrict_networks;
  const connected = network.networks.map((n) => n.name);
  const names = [...network.known];
  for (const name of connected) {
    if (name.trim() !== "" && !names.some((n) => sameName(n, name))) names.unshift(name);
  }

  const isAllowed = (name: string) => config.allowed_networks.some((a) => sameName(a, name));
  const isConnected = (name: string) => connected.some((c) => sameName(c, name));

  const setRestrict = (on: boolean) =>
    onSave((n) =>
      on && n.allowed_networks.length === 0
        ? { ...n, restrict_networks: true, allowed_networks: connected.filter((c) => c.trim() !== "") }
        : { ...n, restrict_networks: on },
    );

  const toggle = (name: string) =>
    onSave((n) => {
      const has = n.allowed_networks.some((a) => sameName(a, name));
      return {
        ...n,
        allowed_networks: has ? n.allowed_networks.filter((a) => !sameName(a, name)) : [...n.allowed_networks, name],
      };
    });

  const forget = (name: string) => {
    onSave((n) => ({ ...n, allowed_networks: n.allowed_networks.filter((a) => !sameName(a, name)) }));
    void agentCall(Method.NetworkForget, { name }).catch((err) => console.error("gagal menghapus jaringan", err));
  };

  return (
    <div className="grid gap-3">
      <Toggle
        label={t.general.restrictNetworks}
        hint={restrict ? t.general.restrictOnHint : t.general.restrictOffHint}
        checked={restrict}
        disabled={busy}
        onChange={setRestrict}
      />

      {restrict && (
      <div className="grid gap-2">
        <div>
          <p className="text-sm font-medium text-neutral-900 dark:text-neutral-100">{t.general.allowedNetworks}</p>
          <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">{t.general.allowedListHint}</p>
        </div>

        {names.length === 0 ? (
          <p className="flex items-center gap-2 text-sm text-neutral-600 dark:text-neutral-400">
            <Icon name="wifi" className="size-4" />
            {t.general.allowedEmpty}
          </p>
        ) : (
          <ul className="grid gap-1.5">
            {names.map((name) => {
              const allowed = isAllowed(name);
              const live = isConnected(name);
              return (
                <li
                  key={name}
                  className="flex items-center gap-3 rounded-lg bg-white/60 px-3 py-2 ring-1 ring-black/5 dark:bg-white/5 dark:ring-white/10"
                >
                  <label className="flex min-w-0 flex-1 cursor-pointer items-center gap-3">
                    <input
                      type="checkbox"
                      checked={allowed}
                      disabled={busy}
                      onChange={() => toggle(name)}
                      className="size-4 shrink-0 accent-indigo-600"
                    />
                    <Icon name="wifi" className="size-4 shrink-0 text-neutral-500 dark:text-neutral-400" />
                    <span className="min-w-0 truncate text-sm text-neutral-900 dark:text-neutral-100">{name}</span>
                    {live && (
                      <span className="shrink-0 rounded-full bg-emerald-100 px-2 py-0.5 text-xs text-emerald-800 dark:bg-emerald-500/15 dark:text-emerald-300">
                        {t.general.allowedConnected}
                      </span>
                    )}
                    {!allowed && (
                      <span className="shrink-0 rounded-full bg-amber-100 px-2 py-0.5 text-xs text-amber-800 dark:bg-amber-500/15 dark:text-amber-300">
                        {t.general.allowedBlocked}
                      </span>
                    )}
                  </label>
                  {!live && (
                    <IconButton
                      icon="x"
                      label={fmt(t.general.allowedForget, { name })}
                      disabled={busy}
                      onClick={() => forget(name)}
                      className="size-7 shrink-0 rounded-full"
                    />
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>
      )}
    </div>
  );
}

interface PortFieldProps {
  port: number;
  busy: boolean;
  onSave: (port: number) => Promise<boolean>;
}

function PortField({ port, busy, onSave }: PortFieldProps) {
  const { t } = useI18n();
  const [draft, setDraft] = useState(String(port));

  useEffect(() => setDraft(String(port)), [port]);

  const parsed = parsePort(draft);
  const dirty = draft.trim() !== String(port);
  const invalid = dirty && parsed === null;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!dirty || parsed === null) return;
    void onSave(parsed);
  };

  return (
    <form onSubmit={submit}>
      <Field label={t.general.port} hint={t.general.portHint} error={invalid ? t.general.portInvalid : null}>
        {(id, describedBy) => (
          <div className="flex gap-2">
            <input
              id={id}
              value={draft}
              inputMode="numeric"
              maxLength={5}
              aria-describedby={describedBy}
              aria-invalid={invalid || undefined}
              onChange={(e) => setDraft(e.currentTarget.value)}
              className={`${inputClass} w-32 font-mono tabular-nums`}
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
