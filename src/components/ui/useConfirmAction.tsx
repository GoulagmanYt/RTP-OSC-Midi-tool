import { useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "./Button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./dialog";

type Request = { title: string; action: () => void | Promise<unknown>; description: string };
type Translate = (key: string, vars?: Record<string, string | number>) => string;

/** All entry points use the same focused, cancellable destructive action. */
export function useConfirmAction(t: Translate) {
  const [request, setRequest] = useState<Request | null>(null);
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);
  const confirmAction = (title: string, action: Request["action"], description = t("ui.irreversible")) => {
    if (!inFlight.current) setRequest({ title, action, description });
  };
  const run = async () => {
    if (!request || inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    try {
      await request.action();
      setRequest(null);
    } catch (error) {
      console.error("Confirmed action failed", error);
      toast.error(t("ui.actionFailed"));
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };
  const confirmation = request ? (
    <Dialog open onOpenChange={open => { if (!open && !inFlight.current) setRequest(null); }}>
      <DialogContent>
        <DialogHeader><DialogTitle>{request.title}</DialogTitle><DialogDescription>{request.description}</DialogDescription></DialogHeader>
        <DialogFooter className="gap-2">
          <Button variant="outline" disabled={busy} autoFocus onClick={() => setRequest(null)}>{t("ui.cancel")}</Button>
          <Button variant="destructive" disabled={busy} aria-busy={busy} onClick={() => void run()}>{request.title}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  ) : null;
  return { confirmAction, confirmation };
}
