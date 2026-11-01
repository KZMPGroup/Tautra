const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["year", 365 * 24 * 3600],
  ["month", 30 * 24 * 3600],
  ["week", 7 * 24 * 3600],
  ["day", 24 * 3600],
  ["hour", 3600],
  ["minute", 60],
  ["second", 1],
];

export function relativeTime(unixMs: number, now: number, locale: string): string {
  const diffSec = Math.round((unixMs - now) / 1000);
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  for (const [unit, size] of UNITS) {
    if (Math.abs(diffSec) >= size || unit === "second") {
      return rtf.format(Math.round(diffSec / size), unit);
    }
  }
  return rtf.format(0, "second");
}

export function dateTime(unixMs: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" }).format(
    new Date(unixMs),
  );
}

export function duration(ms: number, locale: string): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const part = (value: number, unit: "hour" | "minute" | "second") =>
    new Intl.NumberFormat(locale, { style: "unit", unit, unitDisplay: "short" }).format(value);
  if (h > 0) return `${part(h, "hour")} ${part(m, "minute")}`;
  if (m > 0) return `${part(m, "minute")} ${part(s, "second")}`;
  return part(s, "second");
}

export function countdown(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

export function latency(rttMs: number | null, locale: string, unknown: string): string {
  if (rttMs === null || !Number.isFinite(rttMs)) return unknown;
  const digits = rttMs < 1 ? 2 : rttMs < 10 ? 1 : 0;
  const value = new Intl.NumberFormat(locale, {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(rttMs);
  return `${value} ms`;
}

export function shortFingerprint(fingerprint: string): string {
  const groups = fingerprint.split(/[\s:]+/).filter(Boolean);
  return groups.length > 4 ? `${groups.slice(0, 4).join(" ")} …` : fingerprint;
}

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB"];

export function bytes(value: number, locale: string): string {
  let n = Math.max(0, value);
  let unit = 0;
  while (n >= 1024 && unit < BYTE_UNITS.length - 1) {
    n /= 1024;
    unit += 1;
  }
  const digits = unit === 0 || n >= 100 ? 0 : 1;
  const text = new Intl.NumberFormat(locale, { maximumFractionDigits: digits }).format(n);
  return `${text} ${BYTE_UNITS[unit]}`;
}
