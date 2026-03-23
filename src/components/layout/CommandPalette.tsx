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

interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export default function CommandPalette({ open, onOpenChange }: CommandPaletteProps) {
  const navigate = useNavigate();
  const { toggleBridge, status, reloadConfig } = useBridge();
  const { t } = useI18n();

  const runCommand = (command: () => void | Promise<void>) => {
    onOpenChange(false);
    Promise.resolve(command()).catch((e) => {
      const message = e instanceof Error ? e.message : String(e);
      toast.error(message || t("toasts.bridge.bridgeActionFailed"));
    });
  };

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange}>
      <CommandInput placeholder={t("command.placeholder")} />
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
          <CommandItem onSelect={() => runCommand(() => navigate("/logs"))}>
            <ListRestart className="mr-2 h-4 w-4" />
            <span>{t("nav.logs")}</span>
          </CommandItem>
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
              })
            }
          >
            <RotateCcw className="mr-2 h-4 w-4" />
            <span>{t("actions.restartRtp")}</span>
          </CommandItem>
          <CommandItem
            onSelect={() =>
              runCommand(async () => {
                await openVstUi();
              })
            }
          >
            <AudioLines className="mr-2 h-4 w-4" />
            <span>{t("actions.openVstUi")}</span>
          </CommandItem>
          <CommandItem
            onSelect={() =>
              runCommand(async () => {
                await resetConfigDefaults();
                await reloadConfig();
              })
            }
          >
            <Settings className="mr-2 h-4 w-4" />
            <span>{t("actions.resetConfig")}</span>
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
