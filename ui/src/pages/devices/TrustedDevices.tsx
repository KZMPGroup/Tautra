import { useState } from "react";
import { Button, IconButton } from "../../components/Button";
import { Card } from "../../components/Card";
import { ErrorMessage } from "../../components/Feedback";
import { Icon } from "../../components/Icon";
import { fmt, useI18n } from "../../i18n";
import { agentCall, Method, useAction, useNow } from "../../lib/agent";
import { dateTime, relativeTime, shortFingerprint } from "../../lib/format";
import type { AgentStatus, TrustedDevice } from "../../lib/types";

type Presence = "connected" | "online" | "offline";

const PRESENCE_DOT: Record<Presence, string> = {
  connected: "bg-emerald-500",
  online: "bg-sky-500",
  offline: "bg-neutral-400",
};

export function TrustedDevices({ status }: { status: AgentStatus }) {
  const { t } = useI18n();

  const presence = (device: TrustedDevice): Presence => {
    const c = status.connection;
    if (c.state === "connected" && c.device_id === device.device_id) return "connected";
    return status.discovered.some((d) => d.device_id === device.device_id) ? "online" : "offline";
  };

  return (
    <Card title={t.devices.trustedTitle}>
      {status.trusted.length === 0 ? (
        <p className="text-sm text-neutral-600 dark:text-neutral-400">{t.devices.trustedEmpty}</p>
      ) : (
        <ul className="grid divide-y divide-black/5 dark:divide-white/10">
          {status.trusted.map((device) => (
            <TrustedRow key={device.device_id} device={device} presence={presence(device)} />
          ))}
        </ul>
      )}
    </Card>
  );
}

function TrustedRow({ device, presence }: { device: TrustedDevice; presence: Presence }) {
  const { t, locale } = useI18n();
  const now = useNow(30_000);
  const remove = useAction();
  const [confirming, setConfirming] = useState(false);

  const presenceLabel = {
    connected: t.devices.stateConnected,
    online: t.devices.stateOnline,
    offline: t.devices.stateOffline,
  }[presence];

  return (
    <li className="grid gap-3 py-3 first:pt-0 last:pb-0">
      <div className="flex items-start gap-3">
        <Icon name="devices" className="mt-0.5 size-5 text-neutral-500 dark:text-neutral-400" />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium text-neutral-900 dark:text-neutral-100">{device.device_name}</p>
          <p className="mt-0.5 flex items-center gap-1.5 text-xs text-neutral-600 dark:text-neutral-400">
            <span className={`size-1.5 rounded-full ${PRESENCE_DOT[presence]}`} />
            {presenceLabel}
            <span aria-hidden="true">·</span>
            <span title={device.last_connected_unix_ms ? dateTime(device.last_connected_unix_ms, locale) : undefined}>
              {device.last_connected_unix_ms
                ? fmt(t.devices.lastConnected, { time: relativeTime(device.last_connected_unix_ms, now, locale) })
                : t.devices.neverConnected}
            </span>
          </p>
          <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-500">
            <span title={dateTime(device.paired_unix_ms, locale)}>
              {fmt(t.devices.pairedAt, { time: relativeTime(device.paired_unix_ms, now, locale) })}
            </span>
            <span aria-hidden="true"> · </span>
            <span className="selectable font-mono" title={`${t.devices.fingerprint}: ${device.fingerprint}`}>
              {shortFingerprint(device.fingerprint)}
            </span>
            {device.address && (
              <>
                <span aria-hidden="true"> · </span>
                <span className="selectable font-mono">
                  {fmt(t.devices.manualAddress, { address: device.address })}
                </span>
              </>
            )}
          </p>
        </div>
        {!confirming && (
          <IconButton icon="trash" label={t.common.remove} onClick={() => setConfirming(true)} />
        )}
      </div>

      {confirming && (
        <div className="flex flex-wrap items-center gap-3 rounded-lg bg-red-50 px-3.5 py-2.5 dark:bg-red-500/10">
          <p className="min-w-0 flex-1 text-sm text-red-900 dark:text-red-200">
            {fmt(t.devices.removeConfirm, { name: device.device_name })}
          </p>
          <div className="flex gap-2">
            <Button
              onClick={() => {
                setConfirming(false);
                remove.clear();
              }}
              disabled={remove.busy}
            >
              {t.common.cancel}
            </Button>
            <Button
              variant="danger"
              icon="trash"
              busy={remove.busy}
              onClick={() =>
                void remove.run(() => agentCall(Method.DeviceRemove, { device_id: device.device_id }))
              }
            >
              {t.common.confirmRemove}
            </Button>
          </div>
        </div>
      )}
      {remove.error && <ErrorMessage error={remove.error} onDismiss={remove.clear} />}
    </li>
  );
}
