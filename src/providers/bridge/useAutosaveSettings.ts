import { useEffect, useRef } from "react";
import type { Dispatch, SetStateAction } from "react";
import type { AppConfig } from "../../api";
import { getConfigSaveGeneration, isAcceptedConfigReplacement } from "../../config-persistence";

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
    if (!accepted.current || isAcceptedConfigReplacement(config)) {
      accepted.current = config;
      return;
    }
    if (config === accepted.current) {
      return;
    }

    const generation = getConfigSaveGeneration();
    const shouldSave = () => latest.current === config && getConfigSaveGeneration() === generation;
    const timer = window.setTimeout(() => {
      pendingSave.current = pendingSave.current.then(async () => {
        // A delayed save must never overwrite a newer edit or a rollback.
        if (!shouldSave()) return;
        try {
          const saved = await save(config, shouldSave);
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
