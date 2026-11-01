export const MAX_EDGE_DELAY_MS = 2000;

const MODIFIER_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "AltLeft",
  "AltRight",
  "ShiftLeft",
  "ShiftRight",
  "MetaLeft",
  "MetaRight",
  "OSLeft",
  "OSRight",
]);

const NAMED: Record<string, string> = {
  Escape: "Esc",
  Backspace: "Backspace",
  Tab: "Tab",
  Enter: "Enter",
  NumpadEnter: "Enter",
  Space: "Space",
  Pause: "Pause",
  CapsLock: "CapsLock",
  ScrollLock: "ScrollLock",
  NumLock: "NumLock",
  PrintScreen: "PrintScreen",
  Insert: "Insert",
  Delete: "Delete",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  ArrowUp: "Up",
  ArrowDown: "Down",
  NumpadMultiply: "NumpadMultiply",
  NumpadAdd: "NumpadAdd",
  NumpadSubtract: "NumpadSubtract",
  NumpadDecimal: "NumpadDecimal",
  NumpadDivide: "NumpadDivide",
  Semicolon: ";",
  Equal: "=",
  Comma: ",",
  Minus: "-",
  Period: ".",
  Slash: "/",
  Backquote: "`",
  BracketLeft: "[",
  Backslash: "\\",
  BracketRight: "]",
  Quote: "'",
};

function keyName(code: string): string | null {
  if (MODIFIER_CODES.has(code)) return null;
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit) return digit[1];
  const numpad = /^Numpad([0-9])$/.exec(code);
  if (numpad) return `Numpad${numpad[1]}`;
  const fn = /^F([1-9]|1[0-9]|2[0-4])$/.exec(code);
  if (fn) return `F${fn[1]}`;
  return NAMED[code] ?? null;
}

export function hotkeyFromEvent(e: KeyboardEvent): string | null {
  const key = keyName(e.code);
  if (!key) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Win");
  parts.push(key);
  return parts.join("+");
}

function normalize(hotkey: string): string {
  const parts = hotkey
    .split("+")
    .map((p) => p.trim().toLowerCase())
    .filter(Boolean);
  const order = ["ctrl", "control", "alt", "shift", "win", "meta", "super"];
  const canonical = (p: string) => (p === "control" ? "ctrl" : p === "meta" || p === "super" ? "win" : p);
  const mods = parts.filter((p) => order.includes(p)).map(canonical).sort();
  const rest = parts.filter((p) => !order.includes(p));
  return [...mods, ...rest].join("+");
}

export function sameHotkey(a: string, b: string): boolean {
  if (!a.trim() || !b.trim()) return false;
  return normalize(a) === normalize(b);
}

export function parseEdgeDelay(raw: string): number | null {
  if (!/^\d{1,4}$/.test(raw.trim())) return null;
  const value = Number(raw.trim());
  return value <= MAX_EDGE_DELAY_MS ? value : null;
}
