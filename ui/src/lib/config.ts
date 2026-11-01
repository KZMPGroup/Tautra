import { useCallback } from "react";
import { agentCall, Method, useAction, useAgent } from "./agent";
import type { Config } from "./types";

export const MAX_DEVICE_NAME_LEN = 63;

export function useConfigSaver() {
  const { config } = useAgent();
  const action = useAction();
  const { run } = action;

  const save = useCallback(
    (patch: (current: Config) => Config): Promise<boolean> => {
      if (!config) return Promise.resolve(false);
      return run(() => agentCall<Config>(Method.ConfigSet, patch(structuredClone(config))));
    },
    [config, run],
  );

  return { config, save, busy: action.busy, error: action.error, clear: action.clear };
}

export function deviceNameProblem(name: string): "too_long" | "control" | null {
  const trimmed = name.trim();
  if ([...trimmed].length > MAX_DEVICE_NAME_LEN) return "too_long";
  if (/[\u0000-\u001f\u007f]/.test(trimmed)) return "control";
  return null;
}

export function parsePort(raw: string): number | null {
  if (!/^\d{1,5}$/.test(raw.trim())) return null;
  const port = Number(raw.trim());
  return port >= 1 && port <= 65535 ? port : null;
}
