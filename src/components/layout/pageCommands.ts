export interface PageCommand { id: string; label: string; element: HTMLElement }

function labelFor(element: HTMLElement): string {
  const labelledBy = element.getAttribute("aria-labelledby");
  if (labelledBy) {
    const label = labelledBy.split(/\s+/).map(id => document.getElementById(id)?.textContent ?? "").join(" ").trim();
    if (label) return label;
  }
  return (element.getAttribute("aria-label")
    || (element instanceof HTMLInputElement || element instanceof HTMLSelectElement || element instanceof HTMLTextAreaElement
      ? Array.from(element.labels ?? []).map(label => label.textContent).join(" ")
      : element.textContent)
    || element.getAttribute("title") || "").replace(/\s+/g, " ").trim();
}

// One snapshot when opening the palette; no observers or idle polling.
export function collectPageCommands(): PageCommand[] {
  return Array.from(document.querySelectorAll<HTMLElement>(
    "main button, main input:not([type=hidden]), main select, main textarea, main summary, header button",
  )).filter(element => !element.closest('[hidden], [data-command-exclude]') && !element.matches(":disabled"))
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
