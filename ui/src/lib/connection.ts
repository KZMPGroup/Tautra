import { fmt } from "../i18n";
import type { Strings } from "../i18n/strings";
import type { LinkSnapshot } from "./types";

export type Tone = "ok" | "busy" | "idle" | "warn" | "error";

export interface ConnectionView {
  label: string;
  tone: Tone;
  hint: string | null;
}

export function describeConnection(snapshot: LinkSnapshot, t: Strings): ConnectionView {
  const status = snapshot.status;
  if (!snapshot.connected || !status) {
    return { label: t.status.offline, tone: "error", hint: snapshot.error };
  }
  const c = status.connection;
  switch (c.state) {
    case "connected":
      return { label: fmt(t.status.connected, { name: c.device_name }), tone: "ok", hint: null };
    case "connecting":
      return {
        label: fmt(t.status.connecting, { name: c.device_name }),
        tone: "busy",
        hint: fmt(t.home.connectingHint, { attempt: c.attempt }),
      };
    case "searching":
      return { label: t.status.searching, tone: "busy", hint: t.home.searchingHint };
    case "paused":
      return { label: t.status.paused, tone: "warn", hint: t.home.pausedHint };
    case "idle":
      return {
        label: c.reason === "starting" ? t.status.starting : t.status.idle,
        tone: c.reason === "network_not_allowed" || c.reason === "no_network" ? "warn" : "idle",
        hint: t.idle[c.reason],
      };
    case "error":
      return { label: t.status.error, tone: "error", hint: t.errors[c.code] ?? c.message };
  }
}

export const TONE_DOT: Record<Tone, string> = {
  ok: "bg-emerald-500",
  busy: "bg-sky-500 animate-pulse",
  idle: "bg-neutral-400",
  warn: "bg-amber-500",
  error: "bg-red-500",
};
