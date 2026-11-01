import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { setTrayStrings, useAgent } from "../lib/agent";
import type { UiError } from "../lib/types";
import en, { type Strings } from "./en";
import { intlLocale, languageInfo, loadStrings, rememberLanguage, resolveLanguage } from "./languages";

export interface LoadedLanguage {
  code: string;
  t: Strings;
}

interface I18n {
  code: string;
  system: string;
  locale: string;
  dir: "ltr" | "rtl";
  t: Strings;
}

const I18nContext = createContext<I18n>({ code: "en", system: "en", locale: "en", dir: "ltr", t: en });

export function fmt(text: string, vars: Record<string, string | number>): string {
  return text.replace(/\{(\w+)\}/g, (match, key: string) =>
    key in vars ? String(vars[key]) : match,
  );
}

export function I18nProvider({
  system,
  initial,
  children,
}: {
  system: string;
  initial: LoadedLanguage;
  children: ReactNode;
}) {
  const { config } = useAgent();
  const [loaded, setLoaded] = useState(initial);
  const preference = config?.general.language;
  const code = preference === undefined ? loaded.code : resolveLanguage(preference, system);

  useEffect(() => {
    if (loaded.code === code) return;
    let alive = true;
    void loadStrings(code).then((t) => {
      if (alive) setLoaded({ code, t });
    });
    return () => {
      alive = false;
    };
  }, [code, loaded.code]);

  const value = useMemo<I18n>(() => {
    const info = languageInfo(loaded.code);
    return { code: info.code, system, locale: intlLocale(info), dir: info.rtl ? "rtl" : "ltr", t: loaded.t };
  }, [loaded, system]);

  useEffect(() => {
    document.documentElement.lang = value.code;
    document.documentElement.dir = value.dir;
    rememberLanguage(value.code);
    setTrayStrings(value.t.tray).catch((err) => console.error("set_tray_strings gagal", err));
  }, [value]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18n {
  return useContext(I18nContext);
}

export function errorText(t: Strings, code: string): string {
  return t.errors[code] ?? t.errors.internal;
}

export function describeError(t: Strings, error: UiError): { title: string; detail: string } {
  return { title: errorText(t, error.code), detail: error.message };
}
