const KEY = "oscMIDI.reduce-motion.v1";
let sessionPreference: boolean | undefined;

/** This device preference supplements the system's prefers-reduced-motion setting. */
export function readMotionPreference(): boolean {
  if (sessionPreference !== undefined) return sessionPreference;
  try {
    return localStorage.getItem(KEY) === "true";
  } catch {
    return false;
  }
}

export function applyMotionPreference(reduce: boolean): void {
  document.documentElement.classList.toggle("reduce-motion", reduce);
}

export function setMotionPreference(reduce: boolean): void {
  sessionPreference = reduce;
  applyMotionPreference(reduce);
  try {
    localStorage.setItem(KEY, String(reduce));
  } catch { /* The current session still respects the choice without storage. */ }
}
