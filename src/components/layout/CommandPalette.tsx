import { useNavigate } from "react-router-dom";
import {
  Settings,
  LayoutDashboard,
  Cable,
  RadioTower,
  Globe2,
  AudioLines,
  ListRestart,
  Power,
  RotateCcw,
} from "lucide-react";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "../ui/command";
import { openVstUi, resetConfigDefaults, restartRtp } from "../../api";
import { useBridge } from "../../providers/BridgeProvider";
import { useI18n } from "../../providers/LanguageProvider";
import { toast } from "sonner";
import { getErrorMessage } from "../../api-errors";
import { isDeveloperMode } from "../../developer-mode";
import { useConfirmAction } from "../ui/useConfirmAction";

interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export default function CommandPalette({ open, onOpenChange }: CommandPaletteProps) {
  const navigate = useNavigate();
  const { toggleBridge, status, reloadConfig, refreshStatus, config } = useBridge();
  const { t } = useI18n();
  const { confirmAction, confirmation } = useConfirmAction(t);

  const runCommand = (command: () => void | Promise<void>) => {
    onOpenChange(false);
    Promise.resolve()
      .then(() => command())
      .catch((e) => {
        const message = getErrorMessage(e);
        toast.error(message || t("toasts.bridge.bridgeActionFailed"));
      });
  };

  return (
    <>
    {confirmation}
    <CommandDialog open={open} onOpenChange={onOpenChange}>
      <CommandInput placeholder={t("command.placeholder")} aria-label={t("command.placeholder")} />
      <CommandList>
        <CommandEmpty>{t("command.noResults")}</CommandEmpty>
        <CommandGroup heading={t("command.navigation")}>
          <CommandItem onSelect={() => runCommand(() => navigate("/"))}>
            <LayoutDashboard className="mr-2 h-4 w-4" />
            <span>{t("nav.dashboard")}</span>
          </CommandItem>
          <CommandItem onSelect={() => runCommand(() => navigate("/routing"))}>
            <Cable className="mr-2 h-4 w-4" />
            <span>{t("nav.routingMidi")}</span>
          </CommandItem>
          <CommandItem onSelect={() => runCommand(() => navigate("/osc"))}>
            <RadioTower className="mr-2 h-4 w-4" />
            <span>{t("nav.osc")}</span>
          </CommandItem>
          <CommandItem onSelect={() => runCommand(() => navigate("/rtp"))}>
            <Globe2 className="mr-2 h-4 w-4" />
            <span>{t("nav.rtpMidi")}</span>
          </CommandItem>
          <CommandItem onSelect={() => runCommand(() => navigate("/audio"))}>
            <AudioLines className="mr-2 h-4 w-4" />
            <span>{t("nav.audioVst")}</span>
          </CommandItem>
          {isDeveloperMode(config) && <CommandItem onSelect={() => runCommand(() => navigate("/logs"))}>
            <ListRestart className="mr-2 h-4 w-4" />
            <span>{t("nav.logs")}</span>
          </CommandItem>}
          <CommandItem onSelect={() => runCommand(() => navigate("/settings"))}>
            <Settings className="mr-2 h-4 w-4" />
            <span>{t("nav.settings")}</span>
          </CommandItem>
        </CommandGroup>
        <CommandSeparator />
        <CommandGroup heading={t("command.actions")}>
          <CommandItem onSelect={() => runCommand(() => toggleBridge())}>
            <Power className="mr-2 h-4 w-4" />
            <span>{status?.running ? t("actions.stopBridge") : t("actions.startBridge")}</span>
          </CommandItem>
          <CommandItem
            onSelect={() =>
              runCommand(async () => {
                await restartRtp();
                await refreshStatus();
              })
            }
          >
            <RotateCcw className="mr-2 h-4 w-4" />
            <span>{t("actions.restartRtp")}</span>
          </CommandItem>
          <CommandItem
            onSelect={() =>
              runCommand(async () => {
                const wasDeferredByGameMode = Boolean(config?.audio.gameMode && !status?.realtime?.worker.editorOpen);
                await openVstUi();
                await refreshStatus();
                if (wasDeferredByGameMode) {
                  toast.success(t("audio.deferredOperations"));
                } else {
                  toast.success(t("toasts.audio.vstOpened"));
                }
              })
            }
          >
            <AudioLines className="mr-2 h-4 w-4" />
            <span>{t("actions.openVstUi")}</span>
          </CommandItem>
          <CommandItem
            onSelect={() =>
              runCommand(async () => {
                confirmAction(t("ui.resetConfiguration"), async () => {
                  const accepted = await resetConfigDefaults();
                  await reloadConfig(accepted);
                });
              })
            }
          >
            <Settings className="mr-2 h-4 w-4" />
            <span>{t("ui.resetConfiguration")}</span>
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
    </>
  );
}
