import { useEffect, useState } from "react";
import { Button } from "../../components/Button";
import { Card, Detail } from "../../components/Card";
import { ErrorMessage } from "../../components/Feedback";
import { useI18n } from "../../i18n";
import { agentCall, Method, openFolder, quitApp, useAction } from "../../lib/agent";
import { dateTime } from "../../lib/format";
import type { AgentStatus, CompatReport, DiagnosticsInfo, UiInfo } from "../../lib/types";

const USAGE_POLL_MS = 3000;

function percent(value: number | null | undefined, locale: string): string {
  if (value == null) return "—";
  return `${value.toLocaleString(locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 })} %`;
}

function megabytes(value: number | null | undefined, locale: string): string {
  if (value == null) return "—";
  return `${value.toLocaleString(locale, { minimumFractionDigits: 1, maximumFractionDigits: 1 })} MB`;
}

export function AboutSection({ status, info }: { status: AgentStatus; info: UiInfo }) {
  const { t, locale } = useI18n();
  const [diag, setDiag] = useState<DiagnosticsInfo | null>(null);
  const [compat, setCompat] = useState<CompatReport | null>(null);
  const load = useAction();
  const folder = useAction();
  const restart = useAction();
  const quit = useAction();

  useEffect(() => {
    void load.run(async () => {
      setDiag(await agentCall<DiagnosticsInfo>(Method.DiagnosticsGet));
      setCompat(await agentCall<CompatReport>(Method.CompatGet));
    });
  }, [load.run, status.started_unix_ms]);

  useEffect(() => {
    let alive = true;
    const timer = window.setInterval(() => {
      agentCall<DiagnosticsInfo>(Method.DiagnosticsGet)
        .then((next) => alive && setDiag(next))
        .catch(() => undefined);
    }, USAGE_POLL_MS);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  const error = load.error ?? folder.error ?? restart.error ?? quit.error;
  const clearError = () => {
    load.clear();
    folder.clear();
    restart.clear();
    quit.clear();
  };

  return (
    <Card title={t.general.aboutSection}>
      <div className="grid gap-5">
        <dl className="grid grid-cols-2 gap-4 lg:grid-cols-4">
          <Detail label={t.general.uiVersion}>{info.version || "—"}</Detail>
          <Detail label={t.general.agentVersion}>{status.agent_version}</Detail>
          <Detail label={t.general.build}>
            {status.build ? dateTime(status.build * 1000, locale) : "—"}
          </Detail>
          <Detail label={t.general.protocolVersion}>{status.protocol_version}</Detail>
          <Detail label={t.general.agentPid}>{diag?.agent_pid ?? "—"}</Detail>
          <Detail label={t.general.agentCpu}>{percent(diag?.cpu_percent, locale)}</Detail>
          <Detail label={t.general.agentMemory}>{megabytes(diag?.working_set_mb, locale)}</Detail>
          <Detail label={t.general.agentPrivate}>{megabytes(diag?.private_mb, locale)}</Detail>
          <Detail label={t.general.osVersion}>{compat?.os_version ?? "—"}</Detail>
          <Detail label={t.general.arch}>{compat?.arch ?? "—"}</Detail>
        </dl>

        {compat && !compat.supported && compat.warnings.length > 0 && (
          <ul className="grid gap-2">
            {compat.warnings.map((w, i) => (
              <li
                key={i}
                className="rounded-md bg-amber-50 px-3 py-2 text-xs text-amber-800 ring-1 ring-amber-200 dark:bg-amber-500/10 dark:text-amber-200 dark:ring-amber-500/20"
              >
                {w}
              </li>
            ))}
          </ul>
        )}

        {diag && (
          <dl className="grid gap-3">
            <PathRow label={t.general.configPath} path={diag.config_path} />
            <PathRow label={t.general.logDir} path={diag.log_dir} />
            <PathRow label={t.general.dataDir} path={diag.data_dir} />
          </dl>
        )}

        <div className="flex flex-wrap gap-2">
          <Button
            icon="folder"
            disabled={!diag?.log_dir}
            busy={folder.busy}
            onClick={() => diag && void folder.run(() => openFolder(diag.log_dir))}
          >
            {t.general.openLogs}
          </Button>
          <Button
            icon="folder"
            disabled={!diag?.data_dir}
            busy={folder.busy}
            onClick={() => diag && void folder.run(() => openFolder(diag.data_dir))}
          >
            {t.general.openData}
          </Button>
        </div>

        {error && <ErrorMessage error={error} onDismiss={clearError} />}

        <div className="grid gap-4 border-t border-black/5 pt-4 sm:grid-cols-2 dark:border-white/10">
          <div className="grid content-start gap-2">
            <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.general.restartAgentHint}</p>
            <div>
              <Button
                icon="refresh"
                busy={restart.busy}
                onClick={() => void restart.run(() => agentCall(Method.AgentShutdown))}
              >
                {t.general.restartAgent}
              </Button>
            </div>
          </div>
          <div className="grid content-start gap-2">
            <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.general.quitHint}</p>
            <div>
              <Button variant="danger" icon="power" busy={quit.busy} onClick={() => void quit.run(quitApp)}>
                {t.general.quit}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </Card>
  );
}

function PathRow({ label, path }: { label: string; path: string }) {
  return (
    <div className="min-w-0">
      <dt className="text-xs text-neutral-500 dark:text-neutral-400">{label}</dt>
      <dd className="selectable mt-0.5 break-all font-mono text-xs text-neutral-800 dark:text-neutral-200">
        {path || "—"}
      </dd>
    </div>
  );
}
