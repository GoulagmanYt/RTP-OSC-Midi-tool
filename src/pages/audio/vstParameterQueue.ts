type PendingParameter = {
  owner: symbol;
  value: number;
  onError: (error: unknown) => void;
};

// Shared by controller instances so navigation cannot let older edits overtake
// newer ones. Only the latest unsent value for each parameter is retained.
export function createVstParameterQueue(send: (index: number, value: number) => Promise<void>) {
  const pending = new Map<number, PendingParameter>();
  let inFlight = false;
  let target: string | undefined;

  async function drain() {
    if (inFlight) return;
    inFlight = true;
    try {
      while (pending.size > 0) {
        const next = pending.entries().next().value;
        if (!next) break;
        const [index, entry] = next;
        pending.delete(index);
        try {
          await send(index, entry.value);
        } catch (error) {
          entry.onError(error);
        }
      }
    } finally {
      inFlight = false;
    }
  }

  return {
    setTarget(next: string) {
      if (target !== next) pending.clear();
      target = next;
    },
    cancel(owner: symbol) {
      for (const [index, entry] of pending) {
        if (entry.owner === owner) pending.delete(index);
      }
    },
    enqueue(owner: symbol, index: number, value: number, onError: PendingParameter["onError"]) {
      pending.set(index, { owner, value, onError });
      void drain();
    },
  };
}
