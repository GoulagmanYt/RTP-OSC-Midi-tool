import type { CSSProperties } from "react";
import { Check } from "lucide-react";
import type { UiConfig } from "../../api-types";
import { useI18n } from "../../providers/LanguageProvider";
import { ACCENT_CHOICES, DARK_BACKGROUNDS, LIGHT_BACKGROUNDS } from "../../themes";

type Props = {
  ui?: UiConfig;
  disabled: boolean;
  onChange: (patch: Partial<UiConfig>) => Promise<void>;
};

export function AppearanceColors({ ui, disabled, onChange }: Props) {
  const { t } = useI18n();
  const groups = [
    { field: "accentColor" as const, title: "accentTitle", fallback: "blue",
      choices: ACCENT_CHOICES.map(choice => ({ id: choice.id, color: choice.light, label: `settings.theme.accent.${choice.id}` })) },
    { field: "lightBackground" as const, title: "lightBackground", fallback: "default",
      choices: LIGHT_BACKGROUNDS.map(choice => ({ ...choice, label: `settings.theme.background.${choice.label}` })) },
    { field: "darkBackground" as const, title: "darkBackground", fallback: "default",
      choices: DARK_BACKGROUNDS.map(choice => ({ ...choice, label: `settings.theme.background.${choice.label}` })) },
  ];

  return (
    <div className="appearance-colors space-y-5 border-t border-border pt-6">
      <p className="text-sm text-muted-foreground">{t("settings.theme.colorsHint")}</p>
      {groups.map(group => {
        const saved = ui?.[group.field];
        const selected = group.choices.some(choice => choice.id === saved) ? saved : group.fallback;
        return (
          <fieldset key={group.field} disabled={disabled}>
            <legend className="mb-2 text-sm font-medium">{t(`settings.theme.${group.title}`)}</legend>
            <div className="flex flex-wrap gap-x-3 gap-y-2">
              {group.choices.map(choice => (
                <label key={choice.id} className="color-choice cursor-pointer" style={{ "--swatch-color": choice.color } as CSSProperties}>
                  <input type="radio" name={group.field} className="sr-only" checked={selected === choice.id} onChange={() => void onChange({ [group.field]: choice.id })} />
                  <span className="color-choice-swatch" aria-hidden="true"><Check className="h-3.5 w-3.5" strokeWidth={2.5} /></span>
                  <span className="text-xs">{t(choice.label)}</span>
                </label>
              ))}
            </div>
          </fieldset>
        );
      })}
    </div>
  );
}
