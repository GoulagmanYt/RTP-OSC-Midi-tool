import { useEffect, useRef } from "react";
import type { AppConfig } from "../../api";

type AutosaveOptions = {
  config: AppConfig | null;
  save: (config: AppConfig) => Promise<void>;
};

export function useAutosaveSettings({ config, save }: AutosaveOptions) {
  const hasHydrated = useRef(false);
  const saveTimer = useRef<number | null>(null);

  useEffect(() => {
    if (!config) {
      return;
    }
    if (!hasHydrated.current) {
      hasHydrated.current = true;
      return;
    }

    if (saveTimer.current) {
      window.clearTimeout(saveTimer.current);
    }

    saveTimer.current = window.setTimeout(async () => {
      try {
        await save(config);
      } catch (error) {
        console.error("Auto-save failed", error);
      }
    }, 350);

    return () => {
      if (saveTimer.current) {
        window.clearTimeout(saveTimer.current);
      }
    };
  }, [config, save]);
}
