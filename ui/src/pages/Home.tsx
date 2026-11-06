import { useCallback, useEffect, useState } from "react";
import { AgentOfflineCard } from "../components/AgentOfflineCard";
import { Button } from "../components/Button";
import { Card, Detail } from "../components/Card";
import { ErrorMessage } from "../components/Feedback";
import { Icon } from "../components/Icon";
import { NoticeList } from "../components/NoticeList";
import { PageHeader } from "../components/PageHeader";
import type { Page } from "../components/Sidebar";
import { fmt, useI18n } from "../i18n";
import { agentCall, Method, useAction, useAgent, useNow } from "../lib/agent";
import { describeConnection, TONE_DOT, type Tone } from "../lib/connection";
import { duration, latency } from "../lib/format";
import type { AgentStatus, KvmStatus, ServiceStatus } from "../lib/types";

export function Home({ onNavigate }: { onNavigate: (page: Page) => void }) {
  const { t } = useI18n();
  const snapshot = useAgent();
  const status = snapshot.connected ? snapshot.status : null;

  return (
    <>
      <PageHeader title={t.nav.home} />
      <div className="grid gap-4">
        {status ? (
          <>
            <ServiceAlertCard />
            <ConnectionCard status={status} onNavigate={onNavigate} />
            <KvmCard kvm={status.kvm} onNavigate={onNavigate} />
            {status.notices.length > 0 && (
              <Card title={t.home.noticesTitle}>
                <NoticeList notices={status.notices} />
              </Card>
            )}
          </>
        ) : (
          <AgentOfflineCard error={snapshot.error} />
        )}
      </div>
    </>
  );
}

function ConnectionCard({ status, onNavigate }: { status: AgentStatus; onNavigate: (page: Page) => void }) {
  const { t, locale } = useI18n();
  const snapshot = useAgent();
  const now = useNow();
  const action = useAction();
  const view = describeConnection(snapshot, t);
  const c = status.connection;

  let button = null;
  if (c.state === "connected" || c.state === "connecting") {
    button = (
      <Button
        icon="unlink"
        busy={action.busy}
        onClick={() => void action.run(() => agentCall(Method.ConnectionDisconnect))}
      >
        {t.home.disconnect}
      </Button>
    );
  } else if (c.state === "paused") {
    button = (
      <Button
        variant="primary"
        icon="link"
        busy={action.busy}
        onClick={() => void action.run(() => agentCall(Method.ConnectionResume))}
      >
        {t.home.resume}
      </Button>
    );
  } else if (c.state === "idle" && c.reason === "no_trusted_devices") {
    button = (
      <Button variant="primary" icon="key" onClick={() => onNavigate("devices")}>
        {t.home.pairDevice}
      </Button>
    );
  } else if (c.state === "idle" && (c.reason === "network_not_allowed" || c.reason === "auto_connect_disabled")) {
    button = (
      <Button icon="settings" onClick={() => onNavigate("general")}>
        {t.home.openSettings}
      </Button>
    );
  }

  return (
    <Card>
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex min-w-0 items-start gap-3">
          <span className={`mt-2 size-3 shrink-0 rounded-full ${TONE_DOT[view.tone]}`} />
          <div className="min-w-0">
            <p className="text-xs font-medium uppercase tracking-wide text-neutral-500 dark:text-neutral-400">
              {t.home.connectionTitle}
            </p>
            <p className="font-display text-xl font-semibold text-neutral-900 dark:text-neutral-50">
              {view.label}
            </p>
            {view.hint && <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">{view.hint}</p>}
          </div>
        </div>
        {button}
      </div>

      {c.state === "connected" && (
        <dl className="mt-5 grid grid-cols-3 gap-4 border-t border-black/5 pt-4 dark:border-white/10">
          <Detail label={t.home.latency}>{latency(c.rtt_ms, locale, "—")}</Detail>
          <Detail label={t.home.address}>{c.address}</Detail>
          <Detail label={t.home.duration}>{duration(now - c.since_unix_ms, locale)}</Detail>
        </dl>
      )}

      {action.error && (
        <div className="mt-4">
          <ErrorMessage error={action.error} onDismiss={action.clear} />
        </div>
      )}
    </Card>
  );
}

function KvmCard({ kvm, onNavigate }: { kvm: KvmStatus; onNavigate: (page: Page) => void }) {
  const { t } = useI18n();
  const peer = kvm.layout?.devices.find((d) => !d.is_local)?.device_name ?? t.common.unknown;

  let text = t.home.kvmInactive;
  let tone: Tone = "idle";
  if (kvm.state === "waiting") {
    text = t.home.kvmWaiting;
    tone = "busy";
  } else if (kvm.state === "ready" && kvm.role === "input_host") {
    text = fmt(kvm.active === "remote" ? t.home.kvmHostRemote : t.home.kvmHostLocal, { name: peer });
    tone = "ok";
  } else if (kvm.state === "ready" && kvm.role === "remote") {
    text = fmt(kvm.controlled_by_peer ? t.home.kvmRemoteActive : t.home.kvmRemoteIdle, { name: peer });
    tone = "ok";
  }

  return (
    <Card
      title={t.home.kvmTitle}
      actions={
        kvm.state === "ready" ? (
          <Button icon="monitor" onClick={() => onNavigate("layout")}>
            {t.home.kvmEditLayout}
          </Button>
        ) : undefined
      }
    >
      <div className="flex items-start gap-3">
        <span className={`mt-1.5 size-2.5 shrink-0 rounded-full ${TONE_DOT[tone]}`} />
        <p className="min-w-0 flex-1 text-sm text-neutral-700 dark:text-neutral-300">{text}</p>
      </div>
      {(kvm.locked || kvm.gaming) && (
        <div className="mt-3 flex flex-wrap gap-2">
          {kvm.locked && (
            <span className="inline-flex items-center gap-1.5 rounded-full bg-amber-100 px-2.5 py-0.5 text-xs font-medium text-amber-800 dark:bg-amber-500/15 dark:text-amber-300">
              <Icon name="lock" className="size-3.5" />
              {t.home.kvmLocked}
            </span>
          )}
          {kvm.gaming && (
            <span className="inline-flex items-center gap-1.5 rounded-full bg-violet-100 px-2.5 py-0.5 text-xs font-medium text-violet-800 dark:bg-violet-500/15 dark:text-violet-300">
              <Icon name="gamepad" className="size-3.5" />
              {t.home.kvmGaming}
            </span>
          )}
        </div>
      )}
    </Card>
  );
}

function ServiceAlertCard() {
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
    const id = window.setInterval(() => void refresh(), 10_000);
    return () => window.clearInterval(id);
  }, [refresh]);

  if (!svc || (svc.installed && svc.running)) return null;

  const label = svc.installed ? t.input.serviceStateStopped : t.input.serviceStateMissing;

  return (
    <Card
      title={t.input.serviceTitle}
      actions={
        <Button
          variant="primary"
          icon="download"
          busy={action.busy}
          onClick={() => void action.run(async () => setSvc(await agentCall<ServiceStatus>(Method.ServiceInstall)))}
        >
          {t.input.serviceInstall}
        </Button>
      }
    >
      <div className="grid gap-3">
        <div className="flex items-center gap-2 text-sm font-medium text-amber-800 dark:text-amber-300">
          <span className="size-2 rounded-full bg-amber-500" aria-hidden />
          {label}
        </div>
        <p className="text-sm text-neutral-600 dark:text-neutral-400">{t.input.serviceIntro}</p>
        <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.input.serviceAdminNote}</p>
        {action.error && <ErrorMessage error={action.error} onDismiss={action.clear} />}
      </div>
    </Card>
  );
}
