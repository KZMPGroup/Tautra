import en, { type Strings } from "./en";

export const AUTO_LANGUAGE = "auto";

export interface LanguageInfo {
  code: string;
  native: string;
  english: string;
  rtl?: boolean;
  intl?: string;
}

export const LANGUAGES: LanguageInfo[] = [
  { code: "en", native: "English", english: "English" },
  { code: "zh-CN", native: "简体中文", english: "Chinese (Simplified)" },
  { code: "hi", native: "हिन्दी", english: "Hindi" },
  { code: "es", native: "Español", english: "Spanish" },
  { code: "fr", native: "Français", english: "French" },
  { code: "ar", native: "العربية", english: "Arabic", rtl: true },
  { code: "bn", native: "বাংলা", english: "Bengali" },
  { code: "pt-BR", native: "Português (Brasil)", english: "Portuguese (Brazil)" },
  { code: "ru", native: "Русский", english: "Russian" },
  { code: "id", native: "Bahasa Indonesia", english: "Indonesian" },
  { code: "de", native: "Deutsch", english: "German" },
  { code: "ja", native: "日本語", english: "Japanese" },
  { code: "vi", native: "Tiếng Việt", english: "Vietnamese" },
  { code: "tr", native: "Türkçe", english: "Turkish" },
  { code: "ko", native: "한국어", english: "Korean" },
  { code: "jv", native: "Basa Jawa", english: "Javanese", intl: "id" },
  { code: "it", native: "Italiano", english: "Italian" },
  { code: "th", native: "ไทย", english: "Thai" },
  { code: "zh-TW", native: "繁體中文", english: "Chinese (Traditional)" },
  { code: "pl", native: "Polski", english: "Polish" },
  { code: "uk", native: "Українська", english: "Ukrainian" },
  { code: "fil", native: "Filipino", english: "Filipino" },
  { code: "ms", native: "Bahasa Melayu", english: "Malay" },
  { code: "su", native: "Basa Sunda", english: "Sundanese", intl: "id" },
  { code: "nl", native: "Nederlands", english: "Dutch" },
];

const BY_CODE = new Map(LANGUAGES.map((l) => [l.code.toLowerCase(), l]));
const BY_BASE = new Map(LANGUAGES.map((l) => [l.code.toLowerCase().split("-")[0], l] as const).reverse());
const ALIASES: Record<string, string> = { in: "id", tl: "fil" };
const TRADITIONAL = ["hant", "tw", "hk", "mo"];
const STORAGE_KEY = "tautra.language";

const loaders = import.meta.glob<Strings>("./locales/*.ts", { import: "default" });

export function languageInfo(code: string): LanguageInfo {
  return BY_CODE.get(code.toLowerCase()) ?? LANGUAGES[0];
}

function matchTag(tag: string): LanguageInfo | undefined {
  const parts = tag.trim().toLowerCase().replace(/_/g, "-").split("-");
  const base = ALIASES[parts[0]] ?? parts[0];
  if (base === "zh") return BY_CODE.get(parts.some((p) => TRADITIONAL.includes(p)) ? "zh-tw" : "zh-cn");
  return BY_CODE.get([base, ...parts.slice(1)].join("-")) ?? BY_BASE.get(base);
}

export function matchLanguage(tags: readonly string[]): string {
  for (const tag of tags) {
    const found = matchTag(tag);
    if (found) return found.code;
  }
  return "en";
}

export function resolveLanguage(preference: string, system: string): string {
  return preference === AUTO_LANGUAGE ? system : (BY_CODE.get(preference.toLowerCase())?.code ?? system);
}

export function intlLocale(info: LanguageInfo): string {
  return Intl.DateTimeFormat.supportedLocalesOf([info.code]).length > 0 ? info.code : (info.intl ?? "en");
}

export async function loadStrings(code: string): Promise<Strings> {
  const load = loaders[`./locales/${code}.ts`];
  if (!load) return en;
  try {
    const strings = await load();
    return { ...strings, errors: { ...en.errors, ...strings.errors } };
  } catch (err) {
    console.error(`bahasa ${code} gagal dimuat`, err);
    return en;
  }
}

export function rememberedLanguage(): string | null {
  try {
    const code = localStorage.getItem(STORAGE_KEY);
    return code ? (BY_CODE.get(code.toLowerCase())?.code ?? null) : null;
  } catch {
    return null;
  }
}

export function rememberLanguage(code: string): void {
  try {
    localStorage.setItem(STORAGE_KEY, code);
  } catch {
    return;
  }
}
