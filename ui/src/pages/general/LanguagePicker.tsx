import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { inputClass } from "../../components/Field";
import { Icon } from "../../components/Icon";
import { fmt, useI18n } from "../../i18n";
import { AUTO_LANGUAGE, LANGUAGES, languageInfo } from "../../i18n/languages";

const DISPLAY_TAG: Record<string, string> = { "zh-CN": "zh-Hans", "zh-TW": "zh-Hant" };

interface Option {
  code: string;
  label: string;
  labelLang?: string;
  detail: string;
  detailLang?: string;
  search: string;
}

function fold(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

function displayNames(locale: string): Intl.DisplayNames | null {
  try {
    return new Intl.DisplayNames([locale], { type: "language" });
  } catch {
    return null;
  }
}

interface LanguagePickerProps {
  value: string;
  disabled?: boolean;
  onChange: (code: string) => void;
}

export function LanguagePicker({ value, disabled, onChange }: LanguagePickerProps) {
  const { t, locale, system } = useI18n();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const id = useId();

  const options = useMemo<Option[]>(() => {
    const names = displayNames(locale);
    const localized = (code: string, fallback: string) => {
      const tag = DISPLAY_TAG[code] ?? code;
      const name = names?.of(tag);
      return name && name.toLowerCase() !== tag.toLowerCase() ? name : fallback;
    };
    const systemInfo = languageInfo(system);
    const auto = t.general.languageSystem;
    return [
      {
        code: AUTO_LANGUAGE,
        label: auto,
        detail: systemInfo.native,
        detailLang: systemInfo.code,
        search: fold(`${auto} ${systemInfo.native} ${systemInfo.english} windows auto`),
      },
      ...LANGUAGES.map((l) => {
        const name = localized(l.code, l.english);
        return {
          code: l.code,
          label: l.native,
          labelLang: l.code,
          detail: fold(name) === fold(l.native) ? "" : name,
          search: fold(`${l.native} ${name} ${l.english} ${l.code}`),
        };
      }),
    ];
  }, [locale, system, t]);

  const needle = fold(query.trim());
  const visible = needle ? options.filter((o) => o.search.includes(needle)) : options;
  const current = options.find((o) => o.code === value) ?? options[0];
  const activeOption = visible[active];

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    listRef.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [open, active, needle]);

  const show = () => {
    setQuery("");
    setActive(Math.max(0, options.indexOf(current)));
    setOpen(true);
  };

  const close = () => {
    setOpen(false);
    triggerRef.current?.focus();
  };

  const pick = (option: Option) => {
    close();
    if (option.code !== value) onChange(option.code);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => Math.min(i + 1, visible.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => Math.max(i - 1, 0));
    } else if (e.key === "PageDown") {
      e.preventDefault();
      setActive((i) => Math.min(i + 8, visible.length - 1));
    } else if (e.key === "PageUp") {
      e.preventDefault();
      setActive((i) => Math.max(i - 8, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (activeOption) pick(activeOption);
    } else if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Tab") {
      setOpen(false);
    }
  };

  return (
    <div className="grid gap-1.5">
      <p id={`${id}-label`} className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
        {t.general.language}
      </p>
      <div ref={rootRef} className="relative w-full max-w-md">
        <button
          ref={triggerRef}
          type="button"
          disabled={disabled}
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-labelledby={`${id}-label ${id}-value`}
          aria-describedby={`${id}-hint`}
          onClick={() => (open ? setOpen(false) : show())}
          onKeyDown={(e) => {
            if (!open && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
              e.preventDefault();
              show();
            }
          }}
          className={`${inputClass} flex items-center gap-2 text-start`}
        >
          <Icon name="globe" className="size-4 text-neutral-500 dark:text-neutral-400" />
          <span id={`${id}-value`} className="min-w-0 flex-1 truncate">
            <span lang={current.labelLang} dir="auto">
              {current.label}
            </span>
            {current.detail && (
              <span lang={current.detailLang} className="text-neutral-500 dark:text-neutral-400">
                {" · "}
                {current.detail}
              </span>
            )}
          </span>
          <Icon
            name="chevronDown"
            className={`size-4 text-neutral-500 transition-transform dark:text-neutral-400 ${open ? "rotate-180" : ""}`}
          />
        </button>

        {open && (
          <div className="absolute inset-x-0 top-full z-20 mt-1 overflow-hidden rounded-lg bg-white shadow-lg ring-1 ring-black/10 dark:bg-neutral-800 dark:ring-white/10">
            <div className="flex items-center gap-2 border-b border-black/5 px-3 dark:border-white/10">
              <Icon name="search" className="size-4 text-neutral-400" />
              <input
                autoFocus
                role="combobox"
                aria-expanded={true}
                aria-controls={`${id}-list`}
                aria-autocomplete="list"
                aria-activedescendant={activeOption ? `${id}-${activeOption.code}` : undefined}
                aria-label={t.general.languageSearch}
                placeholder={t.general.languageSearch}
                value={query}
                spellCheck={false}
                onChange={(e) => {
                  setQuery(e.currentTarget.value);
                  setActive(0);
                }}
                onKeyDown={onKeyDown}
                className="w-full bg-transparent py-2 text-sm text-neutral-900 placeholder:text-neutral-400 focus:outline-none dark:text-neutral-100 dark:placeholder:text-neutral-500"
              />
            </div>
            <ul
              ref={listRef}
              id={`${id}-list`}
              role="listbox"
              aria-labelledby={`${id}-label`}
              className="max-h-72 overflow-y-auto p-1"
            >
              {visible.map((option, index) => {
                const selected = option.code === current.code;
                return (
                  <li
                    key={option.code}
                    id={`${id}-${option.code}`}
                    data-index={index}
                    role="option"
                    aria-selected={selected}
                    onPointerMove={() => setActive(index)}
                    onPointerDown={(e) => e.preventDefault()}
                    onClick={() => pick(option)}
                    className={`flex cursor-default items-center gap-2 rounded-md px-2.5 py-1.5 text-sm ${
                      index === active ? "bg-black/5 dark:bg-white/10" : ""
                    } ${option.code === AUTO_LANGUAGE ? "mb-1" : ""}`}
                  >
                    <span className="min-w-0 flex-1 truncate">
                      <span
                        lang={option.labelLang}
                        dir="auto"
                        className={selected ? "font-semibold text-indigo-700 dark:text-indigo-300" : "font-medium text-neutral-900 dark:text-neutral-50"}
                      >
                        {option.label}
                      </span>
                      {option.detail && (
                        <span lang={option.detailLang} className="ms-2 text-xs text-neutral-500 dark:text-neutral-400">
                          {option.detail}
                        </span>
                      )}
                    </span>
                    {selected && <Icon name="check" className="size-4 text-indigo-600 dark:text-indigo-400" />}
                  </li>
                );
              })}
              {visible.length === 0 && (
                <li role="presentation" className="px-2.5 py-3 text-sm text-neutral-500 dark:text-neutral-400">
                  {fmt(t.general.languageEmpty, { query: query.trim() })}
                </li>
              )}
            </ul>
          </div>
        )}
      </div>
      <p id={`${id}-hint`} className="text-xs text-neutral-500 dark:text-neutral-400">
        {t.general.languageHint}
      </p>
    </div>
  );
}
