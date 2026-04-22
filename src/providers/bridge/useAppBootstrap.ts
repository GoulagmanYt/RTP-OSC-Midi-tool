import { useEffect } from "react";
import * as api from "../../api";

type BootstrapOptions = {
  setConfig: (config: api.AppConfig) => void;
  setStatus: (status: api.RuntimeStatus) => void;
  setPreflight: (report: api.PreflightReport | null) => void;
  setIsLoading: (loading: boolean) => void;
  refreshStatus: () => Promise<void>;
  refreshLists: () => Promise<void>;
  refreshAudioDevices: (backend?: string | null) => Promise<void>;
};

export function useAppBootstrap({
  setConfig,
  setStatus,
  setPreflight,
  setIsLoading,
  refreshStatus,
  refreshLists,
  refreshAudioDevices,
}: BootstrapOptions) {
  useEffect(() => {
    let cancelled = false;

    const bootstrap = async () => {
      try {
        const config = await api.getConfig();
        if (cancelled) {
          return;
        }

        setConfig(config);
        await Promise.all([
          refreshStatus(),
          refreshLists(),
          config.audio.backend ? refreshAudioDevices(config.audio.backend) : Promise.resolve(),
        ]);

        try {
          const report = await api.preflightCheck();
          if (!cancelled) {
            setPreflight(report);
          }
        } catch (error) {
          console.warn("Preflight check failed at init", error);
        }

        if (config.ui.autoStart) {
          try {
            const status = await api.startBridge(config);
            if (!cancelled) {
              setStatus(status);
            }
          } catch (error) {
            console.error("Auto-start bridge failed", error);
          }
        }
      } catch (error) {
        console.error("Initialization failed", error);
      } finally {
        if (!cancelled) {
          setIsLoading(false);
        }
      }
    };

    bootstrap();

    return () => {
      cancelled = true;
    };
  }, [refreshAudioDevices, refreshLists, refreshStatus, setConfig, setIsLoading, setPreflight, setStatus]);
}
