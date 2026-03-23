import { NavLink } from "react-router-dom";
import { cn } from "../../utils";
import {
  LayoutDashboard,
  Cable,
  RadioTower,
  Globe2,
  AudioLines,
  ListRestart,
  Settings,
  CirclePlay,
  CircleStop,
} from "lucide-react";
import { useBridge } from "../../providers/BridgeProvider";
import { Button } from "../ui/Button";
import { Badge } from "../ui/Badge";
import { useI18n } from "../../providers/LanguageProvider";

export function Sidebar() {
  const { status, toggleBridge, isLoading } = useBridge();
  const { t } = useI18n();

  const navItems = [
    { to: "/", label: t("nav.dashboard"), icon: LayoutDashboard },
    { to: "/routing", label: t("nav.routingMidi"), icon: Cable },
    { to: "/osc", label: t("nav.osc"), icon: RadioTower },
    { to: "/rtp", label: t("nav.rtpMidi"), icon: Globe2 },
    { to: "/audio", label: t("nav.audioVst"), icon: AudioLines },
    { to: "/logs", label: t("nav.logs"), icon: ListRestart },
    { to: "/settings", label: t("nav.settings"), icon: Settings },
  ];

  return (
    <aside className="sidebar flex flex-col h-full glass-surface backdrop-blur-xl border-r">
      <div className="p-6 pb-2">
        <h2 className="font-semibold text-lg tracking-tight">{t("sidebar.navigation")}</h2>
      </div>
      <nav className="flex-1 px-4 space-y-1">
        {navItems.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            className={({ isActive }) =>
              cn(
                "nav-pill group flex items-center gap-3 px-3 py-2 text-sm font-medium rounded-md transition-all duration-200",
                isActive
                  ? "bg-primary/10 text-primary border-l-4 border-primary shadow-[0_12px_40px_-24px_rgba(0,0,0,0.6)]"
                  : "text-muted-foreground hover:bg-muted hover:text-foreground"
              )
            }
          >
            <item.icon className="w-4 h-4 transition-transform duration-200 group-hover:translate-x-0.5" />
            {item.label}
          </NavLink>
        ))}
      </nav>

      <div className="p-4 border-t glass-panel">
        <div className="flex flex-col gap-4">
          <div className="flex items-center justify-between">
            <span className="text-xs font-medium text-muted-foreground">{t("common.status")}</span>
            <Badge variant={status?.running ? "success" : "secondary"}>
              {status?.running ? t("common.running") : t("common.stopped")}
            </Badge>
          </div>
          
          <Button 
            variant={status?.running ? "destructive" : "default"} 
            className="w-full justify-start gap-2"
            onClick={toggleBridge}
            disabled={isLoading}
          >
            {status?.running ? <CircleStop className="w-4 h-4" /> : <CirclePlay className="w-4 h-4" />}
            {status?.running ? t("actions.stopBridge") : t("actions.startBridge")}
          </Button>
        </div>
      </div>
    </aside>
  );
}
