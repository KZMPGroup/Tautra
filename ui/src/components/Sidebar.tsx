import { useEffect, useState } from "react";
import { useI18n } from "../i18n";
import { agentCall, Method, useAgent } from "../lib/agent";
import { describeConnection, TONE_DOT } from "../lib/connection";
import { latency } from "../lib/format";
import type { DiagnosticsInfo } from "../lib/types";
import { Icon, type IconName } from "./Icon";

export type Page = "home" | "devices" | "layout" | "input" | "sharing" | "general";

const ITEMS: { page: Page; icon: IconName }[] = [
  { page: "home", icon: "home" },
  { page: "devices", icon: "devices" },
  { page: "layout", icon: "monitor" },
  { page: "input", icon: "keyboard" },
  { page: "sharing", icon: "clipboard" },
  { page: "general", icon: "settings" },
];

const USAGE_POLL_MS = 3000;

interface Usage {
  cpu: number | null;
  ram: number | null;
}

function useAgentUsage(online: boolean): Usage | null {
  const [usage, setUsage] = useState<Usage | null>(null);

  useEffect(() => {
    if (!online) {
      setUsage(null);
      return;
    }
    let alive = true;
    const load = () => {
      if (document.hidden) return;
      agentCall<DiagnosticsInfo>(Method.DiagnosticsGet)
        .then((d) => alive && setUsage({ cpu: d.cpu_percent, ram: d.working_set_mb }))
        .catch(() => undefined);
    };
    load();
    const timer = window.setInterval(load, USAGE_POLL_MS);
    document.addEventListener("visibilitychange", load);
    return () => {
      alive = false;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", load);
    };
  }, [online]);

  return usage;
}

function number(value: number | null | undefined, digits: number, locale: string): string | null {
  if (value == null || !Number.isFinite(value)) return null;
  return value.toLocaleString(locale, { minimumFractionDigits: digits, maximumFractionDigits: digits });
}

interface SidebarProps {
  page: Page;
  onNavigate: (page: Page) => void;
}

export function Sidebar({ page, onNavigate }: SidebarProps) {
  const { t, locale } = useI18n();
  const snapshot = useAgent();
  const view = describeConnection(snapshot, t);
  const deviceName = snapshot.status?.local_device.device_name;
  const online = snapshot.connected && !!snapshot.status;
  const usage = useAgentUsage(online);
  const connection = snapshot.status?.connection;
  const subtitle =
    connection?.state === "connected"
      ? `${t.home.latency} ${latency(connection.rtt_ms, locale, t.common.unknown)}`
      : view.hint;
  const cpu = number(usage?.cpu, 2, locale);
  const ram = number(usage?.ram, 1, locale);

  return (
    <nav className="flex w-56 shrink-0 flex-col gap-1 px-3 py-4" aria-label="Tautra">
      <div className="mb-4 flex items-center gap-2.5 px-2.5">
        <img src="/logo.svg" alt="" className="size-7" />
        <div className="min-w-0">
          <p className="font-display text-base font-semibold leading-tight text-neutral-900 dark:text-neutral-50">
            Tautra
          </p>
          {deviceName && (
            <p className="truncate text-xs text-neutral-500 dark:text-neutral-400">{deviceName}</p>
          )}
        </div>
      </div>

      {ITEMS.map((item) => {
        const active = item.page === page;
        return (
          <button
            key={item.page}
            type="button"
            aria-current={active ? "page" : undefined}
            onClick={() => onNavigate(item.page)}
            className={`relative flex items-center gap-3 rounded-md px-2.5 py-2 text-left text-sm transition-colors focus-visible:outline-2 focus-visible:outline-indigo-500 ${
              active
                ? "bg-black/5 font-medium text-neutral-900 dark:bg-white/10 dark:text-neutral-50"
                : "text-neutral-700 hover:bg-black/4 dark:text-neutral-300 dark:hover:bg-white/5"
            }`}
          >
            {active && (
              <span className="absolute inset-y-2 left-0 w-1 rounded-full bg-indigo-600 dark:bg-indigo-400" />
            )}
            <Icon name={item.icon} className="size-4.5" />
            {t.nav[item.page]}
          </button>
        );
      })}

      <div className="mt-auto rounded-lg bg-black/4 p-2.5 ring-1 ring-black/5 dark:bg-white/5 dark:ring-white/10">
        <div className="flex items-center gap-2.5">
          <span className="grid size-7 shrink-0 place-items-center rounded-md bg-white shadow-sm ring-1 ring-black/5 dark:bg-white/10 dark:ring-white/10">
            <span className={`size-2.5 rounded-full ${TONE_DOT[view.tone]}`} />
          </span>
          <div className="min-w-0">
            <p
              className="truncate text-sm font-medium leading-tight text-neutral-900 dark:text-neutral-50"
              title={view.label}
            >
              {view.label}
            </p>
            {subtitle && (
              <p className="truncate text-xs text-neutral-500 dark:text-neutral-400" title={subtitle}>
                {subtitle}
              </p>
            )}
          </div>
        </div>
        {online && (
          <dl className="mt-2.5 grid grid-cols-2 gap-2 border-t border-black/5 pt-2.5 dark:border-white/10">
            <UsageStat label="CPU" value={cpu ? `${cpu} %` : "—"} />
            <UsageStat label="RAM" value={ram ? `${ram} MB` : "—"} />
          </dl>
        )}
      </div>
    </nav>
  );
}

function UsageStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <dt className="text-[0.625rem] font-medium uppercase tracking-wide text-neutral-500 dark:text-neutral-400">
        {label}
      </dt>
      <dd className="truncate font-mono text-xs tabular-nums text-neutral-800 dark:text-neutral-200">{value}</dd>
    </div>
  );
}
