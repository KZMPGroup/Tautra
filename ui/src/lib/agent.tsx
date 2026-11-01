import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { LinkSnapshot, UiError, UiInfo } from "./types";

export const Method = {
  StatusGet: "status.get",
  ConfigGet: "config.get",
  ConfigSet: "config.set",
  PairingHost: "pairing.host",
  PairingBrowse: "pairing.browse",
  PairingJoin: "pairing.join",
  PairingCancel: "pairing.cancel",
  DeviceRemove: "device.remove",
  ConnectionDisconnect: "connection.disconnect",
  ConnectionResume: "connection.resume",
  NoticeDismiss: "notice.dismiss",
  NetworkForget: "network.forget",
  DiagnosticsGet: "diagnostics.get",
  CompatGet: "compat.get",
  AgentShutdown: "agent.shutdown",
  LayoutSet: "layout.set",
  LayoutGuide: "layout.guide",
  KvmSetLock: "kvm.set_lock",
  KvmSetGaming: "kvm.set_gaming",
  ServiceStatus: "service.status",
  ServiceInstall: "service.install",
  ServiceUninstall: "service.uninstall",
  TransferCancel: "transfer.cancel",
  TransferReveal: "transfer.reveal",
  TransferClear: "transfer.clear",
} as const;

export type MethodName = (typeof Method)[keyof typeof Method];

const SNAPSHOT_EVENT = "agent://snapshot";

export function toUiError(err: unknown): UiError {
  if (err && typeof err === "object" && "code" in err && "message" in err) {
    const e = err as { code: unknown; message: unknown };
    return { code: String(e.code), message: String(e.message) };
  }
  if (err instanceof Error) return { code: "internal", message: err.message };
  return { code: "internal", message: String(err) };
}

export async function agentCall<T = null>(method: MethodName, params?: unknown): Promise<T> {
  try {
    return await invoke<T>("agent_call", { method, params: params ?? null });
  } catch (err) {
    throw toUiError(err);
  }
}

export async function startAgent(): Promise<void> {
  try {
    await invoke("start_agent");
  } catch (err) {
    throw toUiError(err);
  }
}

export async function openFolder(path: string, create = false): Promise<void> {
  try {
    await invoke("open_folder", { path, create });
  } catch (err) {
    throw toUiError(err);
  }
}

export function getUiInfo(): Promise<UiInfo> {
  return invoke<UiInfo>("ui_info");
}

export function notifyUiReady(): Promise<void> {
  return invoke("ui_ready");
}

export function quitApp(): Promise<void> {
  return invoke("quit_app");
}

const EMPTY: LinkSnapshot = { connected: false, status: null, config: null, error: null };

const AgentContext = createContext<LinkSnapshot>(EMPTY);

export function AgentProvider({ children }: { children: ReactNode }) {
  const [snapshot, setSnapshot] = useState<LinkSnapshot>(EMPTY);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<LinkSnapshot>(SNAPSHOT_EVENT, (event) => {
      if (!disposed) setSnapshot(event.payload);
    })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch((err) => console.error("gagal mendengarkan snapshot agent", err));
    invoke<LinkSnapshot>("agent_snapshot")
      .then((s) => {
        if (!disposed) setSnapshot(s);
      })
      .catch((err) => console.error("gagal membaca snapshot agent", err));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return <AgentContext.Provider value={snapshot}>{children}</AgentContext.Provider>;
}

export function useAgent(): LinkSnapshot {
  return useContext(AgentContext);
}

export function useNow(intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(id);
  }, [intervalMs]);
  return now;
}

export function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<UiError | null>(null);

  const run = useCallback(async (action: () => Promise<unknown>): Promise<boolean> => {
    setBusy(true);
    setError(null);
    try {
      await action();
      return true;
    } catch (err) {
      const e = toUiError(err);
      console.error(e.code, e.message);
      setError(e);
      return false;
    } finally {
      setBusy(false);
    }
  }, []);

  const clear = useCallback(() => setError(null), []);

  return { busy, error, run, clear };
}
