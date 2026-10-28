export interface PageCommand { id: string; label: string; element: HTMLElement }

function labelFor(element: HTMLElement): string {
  const labelledBy = element.getAttribute("aria-labelledby");
  if (labelledBy) {
    const label = labelledBy.split(/\s+/).map(id => document.getElementById(id)?.textContent ?? "").join(" ").trim();
    if (label) return label;
  }
  const externalLabel = element.id ? Array.from(document.querySelectorAll("label"))
    .filter(label => label.htmlFor === element.id).map(label => label.textContent).join(" ").trim() : "";
  return (element.getAttribute("aria-label")
    || externalLabel
    || (element instanceof HTMLInputElement || element instanceof HTMLSelectElement || element instanceof HTMLTextAreaElement
      ? Array.from(element.labels ?? []).map(label => label.textContent).join(" ")
      : element.textContent)
    || element.getAttribute("title") || "").replace(/\s+/g, " ").trim();
}

// One snapshot when opening the palette; no observers or idle polling.
export function collectPageCommands(): PageCommand[] {
  const dialogs = Array.from(document.querySelectorAll<HTMLElement>('[role="dialog"]:not([aria-hidden="true"]):not([data-state="closed"])'));
  const dialog = dialogs[dialogs.length - 1];
  const root = dialog ?? document;
  const selector = dialog ? "button, input:not([type=hidden]), select, textarea, summary"
    : "main button, main input:not([type=hidden]), main select, main textarea, main summary, header button";
  return Array.from(root.querySelectorAll<HTMLElement>(selector)).filter(element => !element.closest('[hidden], [data-command-exclude]') && !element.matches(":disabled"))
    .map((element, index) => ({ id: `page-control-${index}`, label: labelFor(element), element }))
    .filter(command => command.label.length > 0);
}

export function activatePageCommand(command: PageCommand): boolean {
  const element = command.element;
  if (!element.isConnected || element.matches(":disabled")) return false;
  let parent = element.parentElement;
  while (parent) {
    if (parent instanceof HTMLDetailsElement) parent.open = true;
    parent = parent.parentElement;
  }
  element.scrollIntoView?.({ block: "nearest" });
  element.focus();
  if (element.matches('button, summary, input[type=checkbox], input[type=radio]')) element.click();
  return true;
}
