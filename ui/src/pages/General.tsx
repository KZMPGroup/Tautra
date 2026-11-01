import { AgentOfflineCard } from "../components/AgentOfflineCard";
import { PageHeader } from "../components/PageHeader";
import { useI18n } from "../i18n";
import { useAgent } from "../lib/agent";
import type { UiInfo } from "../lib/types";
import { AboutSection } from "./general/AboutSection";
import { ConnectionSection } from "./general/ConnectionSection";
import { DeviceSection } from "./general/DeviceSection";

export function General({ info }: { info: UiInfo }) {
  const { t } = useI18n();
  const snapshot = useAgent();
  const status = snapshot.connected ? snapshot.status : null;
  const config = snapshot.config;

  return (
    <>
      <PageHeader title={t.nav.general} />
      {status && config ? (
        <div className="grid gap-4">
          <DeviceSection config={config} effectiveName={status.local_device.device_name} />
          <ConnectionSection config={config} network={status.network} />
          <AboutSection status={status} info={info} />
        </div>
      ) : (
        <AgentOfflineCard error={snapshot.error} />
      )}
    </>
  );
}
