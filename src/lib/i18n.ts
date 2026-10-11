/**
 * Message catalogs and t() (SPEC §13). `en.json` is the source catalog; other catalogs may be
 * partial and fall back to English per key. Dates and numbers never come from catalogs (Intl).
 */
import en from "../i18n/en.json";

type PluralForms = Partial<Record<Intl.LDMLPluralRule, string>> & { other: string };
type Entry = string | PluralForms;
export type Catalog = Readonly<Record<string, Entry>>;
export type MsgKey = keyof typeof en;
export type MsgParams = Readonly<Record<string, string | number>>;

const catalogs = new Map<string, Catalog>([["en", en]]);
let language = "en";
let active: Catalog = en;
let pluralRules = new Intl.PluralRules("en");
const reported = new Set<string>();

/** Adds or replaces a catalog (future languages, tests). */
export function registerCatalog(lang: string, catalog: Catalog): void {
  catalogs.set(lang, catalog);
}

export function availableLanguages(): string[] {
  return [...catalogs.keys()];
}

/**
 * Selects the catalog for `requested` ("auto" uses the browser/OS language). Tries the full tag,
 * then its base language, then English. Returns the language actually used.
 */
export function setLanguage(requested: string): string {
  const tag = requested === "auto" ? (globalThis.navigator?.language ?? "en") : requested;
  const base = tag.split("-")[0] ?? "en";
  language = catalogs.has(tag) ? tag : catalogs.has(base) ? base : "en";
  active = catalogs.get(language) ?? en;
  pluralRules = new Intl.PluralRules(language);
  return language;
}

/** The message for `key`, with `{name}` placeholders replaced; `count` selects plural forms. */
export function t(key: MsgKey, params?: MsgParams): string {
  let entry: Entry | undefined = active[key];
  if (entry === undefined) {
    entry = (en as Catalog)[key];
    if (!reported.has(key)) {
      reported.add(key);
      console.warn(`i18n: "${key}" missing in "${language}", using English`);
    }
  }
  if (entry === undefined) return key;
  const template = typeof entry === "string" ? entry : (entry[pluralRules.select(Number(params?.count ?? 0))] ?? entry.other);
  return params ? template.replace(/\{(\w+)\}/g, (m, name: string) => (name in params ? String(params[name]) : m)) : template;
}
