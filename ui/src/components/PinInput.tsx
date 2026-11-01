import { useEffect, useRef, type ChangeEvent, type ClipboardEvent, type KeyboardEvent } from "react";
import { fmt, useI18n } from "../i18n";

export const PIN_LENGTH = 6;

interface PinInputProps {
  value: string;
  disabled?: boolean;
  autoFocus?: boolean;
  labelledBy?: string;
  onChange: (value: string) => void;
  onComplete?: (value: string) => void;
}

const onlyDigits = (text: string) => text.replace(/\D/g, "");

export function PinInput({ value, disabled, autoFocus, labelledBy, onChange, onComplete }: PinInputProps) {
  const { t } = useI18n();
  const refs = useRef<(HTMLInputElement | null)[]>([]);
  const current = useRef(value);
  const wasDisabled = useRef(disabled);
  const digits = Array.from({ length: PIN_LENGTH }, (_, i) => value[i] ?? "");

  const focus = (index: number) => {
    const el = refs.current[Math.max(0, Math.min(PIN_LENGTH - 1, index))];
    el?.focus();
    el?.select();
  };

  useEffect(() => {
    current.current = value;
    const active = refs.current.findIndex((el) => el !== null && el === document.activeElement);
    if (active > value.length) focus(value.length);
  }, [value]);

  useEffect(() => {
    if (wasDisabled.current && !disabled && autoFocus) focus(current.current.length);
    wasDisabled.current = disabled;
  }, [disabled, autoFocus]);

  const commit = (next: string, focusIndex: number) => {
    const clean = onlyDigits(next).slice(0, PIN_LENGTH);
    const changed = clean !== current.current;
    current.current = clean;
    if (changed) onChange(clean);
    focus(focusIndex);
    if (changed && clean.length === PIN_LENGTH) onComplete?.(clean);
  };

  const write = (index: number, text: string) => {
    const pin = current.current;
    const at = Math.min(index, pin.length);
    commit(pin.slice(0, at) + text + pin.slice(at + text.length), at + text.length);
  };

  const remove = (index: number) => {
    const pin = current.current;
    if (index < 0) return;
    if (index >= pin.length) {
      focus(index);
      return;
    }
    commit(pin.slice(0, index) + pin.slice(index + 1), index);
  };

  const onKeyDown = (index: number, e: KeyboardEvent<HTMLInputElement>) => {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const filled = index < current.current.length;
    if (/^\d$/.test(e.key)) {
      e.preventDefault();
      write(index, e.key);
    } else if (e.key === "Backspace") {
      e.preventDefault();
      remove(filled ? index : index - 1);
    } else if (e.key === "Delete") {
      e.preventDefault();
      remove(index);
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      focus(index - 1);
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      focus(Math.min(index + 1, current.current.length));
    } else if (e.key === "Home") {
      e.preventDefault();
      focus(0);
    } else if (e.key === "End") {
      e.preventDefault();
      focus(current.current.length);
    } else if (e.key.length === 1) {
      e.preventDefault();
    }
  };

  const onInput = (index: number, e: ChangeEvent<HTMLInputElement>) => {
    const typed = onlyDigits(e.currentTarget.value.replace(digits[index], ""));
    if (typed) write(index, typed);
  };

  const onPaste = (index: number, e: ClipboardEvent<HTMLInputElement>) => {
    e.preventDefault();
    const pasted = onlyDigits(e.clipboardData.getData("text"));
    if (!pasted) return;
    if (pasted.length >= PIN_LENGTH) commit(pasted, PIN_LENGTH - 1);
    else write(index, pasted);
  };

  return (
    <div role="group" aria-labelledby={labelledBy} className="flex gap-2">
      {digits.map((digit, i) => (
        <input
          key={i}
          ref={(el) => {
            refs.current[i] = el;
          }}
          value={digit}
          disabled={disabled}
          autoFocus={autoFocus && i === 0}
          inputMode="numeric"
          pattern="[0-9]*"
          autoComplete="one-time-code"
          aria-label={fmt(t.devices.pinDigit, { index: i + 1 })}
          onFocus={(e) => {
            if (i > current.current.length) focus(current.current.length);
            else e.currentTarget.select();
          }}
          onMouseUp={(e) => e.currentTarget.select()}
          onChange={(e) => onInput(i, e)}
          onKeyDown={(e) => onKeyDown(i, e)}
          onPaste={(e) => onPaste(i, e)}
          className="h-12 w-10 rounded-lg bg-white/90 text-center font-mono text-xl font-semibold text-neutral-900 caret-transparent ring-1 ring-black/10 focus:outline-none focus:ring-2 focus:ring-indigo-500 disabled:opacity-60 dark:bg-black/20 dark:text-neutral-50 dark:ring-white/15"
        />
      ))}
    </div>
  );
}
