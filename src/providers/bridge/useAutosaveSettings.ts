import { useEffect, useRef } from "react";
import type { Dispatch, SetStateAction } from "react";
import type { AppConfig } from "../../api";

type AutosaveOptions = {
  config: AppConfig | null;
  save: (config: AppConfig, shouldSave?: () => boolean) => Promise<boolean | void>;
  setConfig: Dispatch<SetStateAction<AppConfig | null>>;
  onError: (error: unknown) => void;
  loadAccepted?: () => Promise<AppConfig>;
};

export function useAutosaveSettings({ config, save, setConfig, onError, loadAccepted }: AutosaveOptions) {
  const accepted = useRef<AppConfig | null>(null);
  const latest = useRef(config);
  const pendingSave = useRef<Promise<void>>(Promise.resolve());

  useEffect(() => {
    latest.current = config;
    if (!config) {
      return;
    }
    if (!accepted.current) {
      accepted.current = config;
      return;
    }
    if (config === accepted.current) {
      return;
    }

    const timer = window.setTimeout(() => {
      pendingSave.current = pendingSave.current.then(async () => {
        // A delayed save must never overwrite a newer edit or a rollback.
        if (latest.current !== config) return;
        try {
          const saved = await save(config, () => latest.current === config);
          if (saved !== false) accepted.current = config;
        } catch (error) {
          console.error("Auto-save failed", error);
          onError(error);
          let restored = accepted.current;
          if (loadAccepted) {
            try {
              restored = await loadAccepted();
            } catch (loadError) {
              console.error("Failed to reload accepted configuration", loadError);
            }
          }
          accepted.current = restored;
          setConfig((current) => current === config ? restored : current);
        }
      });
    }, 350);

    return () => {
      window.clearTimeout(timer);
    };
  }, [config, save, setConfig, onError, loadAccepted]);
}
