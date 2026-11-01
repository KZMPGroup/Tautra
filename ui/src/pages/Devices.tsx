import { AgentOfflineCard } from "../components/AgentOfflineCard";
import { Card, Detail } from "../components/Card";
import { PageHeader } from "../components/PageHeader";
import { useI18n } from "../i18n";
import { useAgent } from "../lib/agent";
import type { LocalDevice } from "../lib/types";
import { PairCard } from "./devices/PairCard";
import { TrustedDevices } from "./devices/TrustedDevices";

export function Devices() {
  const { t } = useI18n();
  const snapshot = useAgent();
  const status = snapshot.connected ? snapshot.status : null;

  return (
    <>
      <PageHeader title={t.nav.devices} />
      {status ? (
        <div className="grid gap-4">
          <TrustedDevices status={status} />
          <PairCard status={status} />
          <LocalDeviceCard device={status.local_device} />
        </div>
      ) : (
        <AgentOfflineCard error={snapshot.error} />
      )}
    </>
  );
}

function LocalDeviceCard({ device }: { device: LocalDevice }) {
  const { t } = useI18n();
  return (
    <Card title={t.devices.localTitle} description={t.devices.localHint}>
      <dl className="grid gap-3">
        <Detail label={t.general.deviceName}>{device.device_name}</Detail>
        <Detail label={t.devices.deviceId}>
          <span className="font-mono text-xs">{device.device_id || "—"}</span>
        </Detail>
        <div className="min-w-0">
          <dt className="text-xs text-neutral-500 dark:text-neutral-400">{t.devices.fingerprint}</dt>
          <dd className="selectable mt-0.5 break-all font-mono text-xs text-neutral-900 dark:text-neutral-100">
            {device.fingerprint || "—"}
          </dd>
        </div>
      </dl>
    </Card>
  );
}
