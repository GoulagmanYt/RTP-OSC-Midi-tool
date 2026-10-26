import { NavLink } from "react-router-dom";
import { AudioLines, Cable, Globe2, LayoutDashboard, ListRestart, RadioTower, Settings, Waypoints } from "lucide-react";
import { useBridge } from "../../providers/BridgeProvider";
import { useI18n } from "../../providers/LanguageProvider";
import { isDeveloperMode } from "../../developer-mode";

export function Sidebar() {
  const { config } = useBridge();
  const { t } = useI18n();
  const groups = [
    { label: "sidebar.workspace", items: [{ to: "/", label: t("nav.dashboard"), icon: LayoutDashboard }] },
    { label: "sidebar.connections", items: [
      { to: "/routing", label: t("nav.routingMidi"), icon: Cable },
      { to: "/rtp", label: t("nav.rtpMidi"), icon: Globe2 },
      { to: "/osc", label: t("nav.osc"), icon: RadioTower },
      { to: "/audio", label: t("nav.audioVst"), icon: AudioLines },
    ] },
    { label: "sidebar.application", items: [
      ...(isDeveloperMode(config) ? [{ to: "/logs", label: t("nav.logs"), icon: ListRestart }] : []),
      { to: "/settings", label: t("nav.settings"), icon: Settings },
    ] },
  ];

  return (
    <aside id="app-sidebar" className="sidebar navigation-material flex h-full flex-col border-r">
      <div className="sidebar-brand flex min-h-20 shrink-0 items-center gap-3 px-5" data-tauri-drag-region>
        <span className="app-mark grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-primary text-primary-foreground" aria-hidden="true"><Waypoints className="h-5 w-5" strokeWidth={1.75} /></span>
        <span className="sidebar-label text-lg font-semibold tracking-tight" data-tauri-drag-region>OSCMidi</span>
      </div>
      <nav className="flex-1 space-y-6 overflow-y-auto px-3 py-3" aria-label={t("sidebar.navigation")}>
        {groups.map((group) => (
          <div key={group.label}>
            <p className="sidebar-heading mb-2 px-3 text-xs font-medium text-muted-foreground">{t(group.label)}</p>
            <div className="space-y-1">
              {group.items.map((item) => (
                <NavLink key={item.to} to={item.to} end={item.to === "/"} className="nav-item" aria-label={item.label} title={item.label}>
                  <item.icon className="h-[1.125rem] w-[1.125rem] shrink-0" strokeWidth={1.75} aria-hidden="true" />
                  <span className="sidebar-label">{item.label}</span>
                </NavLink>
              ))}
            </div>
          </div>
        ))}
      </nav>
    </aside>
  );
}
