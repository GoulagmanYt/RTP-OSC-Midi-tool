import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { Language, TranslationTree } from "../locales/types";

const STORAGE_KEY = "oscMIDI.language";

const localeLoaders: Record<Language, () => Promise<{ default: TranslationTree }>> = {
  en: () => import("../locales/en"),
  fr: () => import("../locales/fr"),
};

type I18nContextValue = {
  language: Language;
  setLanguage: (lang: Language) => void;
  toggleLanguage: () => void;
  t: (key: string, vars?: Record<string, string | number>) => string;
  ready: boolean;
};

const I18nContext = createContext<I18nContextValue | undefined>(undefined);

function resolveValue(source: TranslationTree | undefined, key: string): string | undefined {
  if (!source) return undefined;
  const parts = key.split(".");
  let current: string | TranslationTree | undefined = source;
  for (const part of parts) {
    if (typeof current !== "object" || current === null || !(part in current)) {
      return undefined;
    }
    current = current[part];
  }
  return typeof current === "string" ? current : undefined;
}

function interpolate(template: string, vars?: Record<string, string | number>) {
  if (!vars) return template;
  return template.replace(/\{\{(\w+)\}\}/g, (_, key) => {
    const value = vars[key];
    return value === undefined ? `{{${key}}}` : String(value);
  });
}

function detectLanguage(): Language {
  if (typeof window === "undefined") return "fr";
  const stored = window.localStorage.getItem(STORAGE_KEY);
  if (stored === "en" || stored === "fr") return stored;
  const browser = window.navigator?.language?.toLowerCase() ?? "";
  if (browser.startsWith("fr")) return "fr";
  return "en";
}

export function LanguageProvider({ children }: { children: React.ReactNode }) {
  const [language, setLanguageState] = useState<Language>(detectLanguage);
  const [catalogs, setCatalogs] = useState<Partial<Record<Language, TranslationTree>>>({});
  const [ready, setReady] = useState(false);
  const catalogsRef = useRef(catalogs);

  useEffect(() => {
    catalogsRef.current = catalogs;
  }, [catalogs]);

  const ensureCatalog = useCallback(async (lang: Language) => {
    if (catalogsRef.current[lang]) return catalogsRef.current[lang];
    const module = await localeLoaders[lang]();
    setCatalogs((prev) => {
      if (prev[lang]) return prev;
      const next = { ...prev, [lang]: module.default };
      catalogsRef.current = next;
      return next;
    });
    return module.default;
  }, []);

  const setLanguage = useCallback((next: Language) => {
    setLanguageState(next);
    try {
      window.localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Ignore storage failures (private mode, permissions, etc.)
    }
  }, []);

  const toggleLanguage = useCallback(() => {
    setLanguageState((prev) => {
      const next = prev === "fr" ? "en" : "fr";
      try {
        window.localStorage.setItem(STORAGE_KEY, next);
      } catch {
        // Ignore storage failures (private mode, permissions, etc.)
      }
      return next;
    });
  }, []);

  useEffect(() => {
    let cancelled = false;
    setReady(false);

    const load = async () => {
      await ensureCatalog(language);
      if (language !== "en") {
        void ensureCatalog("en");
      }
      if (!cancelled) {
        setReady(true);
      }
    };

    void load();

    return () => {
      cancelled = true;
    };
  }, [ensureCatalog, language]);

  const t = useCallback(
    (key: string, vars?: Record<string, string | number>) => {
      const value =
        resolveValue(catalogs[language], key) ??
        resolveValue(catalogs.en, key) ??
        key;
      return interpolate(value, vars);
    },
    [catalogs, language]
  );

  const contextValue = useMemo(
    () => ({
      language,
      setLanguage,
      toggleLanguage,
      t,
      ready,
    }),
    [language, ready, setLanguage, t, toggleLanguage]
  );

  useEffect(() => {
    document.documentElement.lang = language;
  }, [language]);

  if (!ready) {
    return (
      <div className="min-h-screen bg-background text-foreground" aria-hidden="true" />
    );
  }

  return <I18nContext.Provider value={contextValue}>{children}</I18nContext.Provider>;
}

export function useI18n() {
  const context = useContext(I18nContext);
  if (!context) {
    throw new Error("useI18n must be used within a LanguageProvider");
  }
  return context;
}

export type { Language };
