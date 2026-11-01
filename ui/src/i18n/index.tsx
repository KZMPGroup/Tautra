import { createContext, useContext, useEffect, useMemo, type ReactNode } from "react";
import { useAgent } from "../lib/agent";
import type { Language, UiError } from "../lib/types";
import { strings, type Strings } from "./strings";

interface I18n {
  lang: Language;
  locale: string;
  t: Strings;
}

const I18nContext = createContext<I18n>({ lang: "id", locale: "id-ID", t: strings.id });

export function fmt(text: string, vars: Record<string, string | number>): string {
  return text.replace(/\{(\w+)\}/g, (match, key: string) =>
    key in vars ? String(vars[key]) : match,
  );
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const { config } = useAgent();
  const lang: Language = config?.general.language ?? "id";

  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);

  const value = useMemo<I18n>(
    () => ({ lang, locale: lang === "id" ? "id-ID" : "en-US", t: strings[lang] }),
    [lang],
  );

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
