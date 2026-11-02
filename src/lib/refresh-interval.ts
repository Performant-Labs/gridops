/**
 * The per-browser Refresh preference (Settings -> Preferences): how often live
 * pages reload their data. Stored like the theme, and safe without storage.
 */
export const REFRESH_STORAGE_KEY = "gridops-refresh-interval";

export type RefreshPreference = "5s" | "10s" | "30s" | "1m" | "off";

export const REFRESH_OPTIONS: ReadonlyArray<{ value: RefreshPreference; label: string; milliseconds: number | null }> = [
  { value: "5s", label: "5 seconds", milliseconds: 5_000 },
  { value: "10s", label: "10 seconds", milliseconds: 10_000 },
  { value: "30s", label: "30 seconds", milliseconds: 30_000 },
  { value: "1m", label: "1 minute", milliseconds: 60_000 },
  { value: "off", label: "Off", milliseconds: null },
];

export const DEFAULT_REFRESH_PREFERENCE: RefreshPreference = "10s";

/**
 * Each page keeps its own interval, scaled by the choice relative to this
 * default, so the default behaves exactly as before the setting existed.
 */
const BASELINE_MILLISECONDS = 10_000;

/**
 * No page polls faster than this. It is the fastest interval GridOps already
 * used (an active workflow run), so no choice adds load the API did not
 * already carry.
 */
export const MIN_REFRESH_MILLISECONDS = 3_000;

export function resolveRefreshPreference(value: unknown): RefreshPreference {
  return REFRESH_OPTIONS.find((option) => option.value === value)?.value ?? DEFAULT_REFRESH_PREFERENCE;
}

export function readRefreshPreference(storage: Pick<Storage, "getItem"> | null): RefreshPreference {
  try {
    return resolveRefreshPreference(storage?.getItem(REFRESH_STORAGE_KEY));
  } catch {
    return DEFAULT_REFRESH_PREFERENCE;
  }
}

/** Returns whether the choice was stored; the setting still applies for this page view when it was not. */
export function writeRefreshPreference(storage: Pick<Storage, "setItem"> | null, preference: RefreshPreference): boolean {
  if (!storage) return false;
  try {
    storage.setItem(REFRESH_STORAGE_KEY, preference);
    return true;
  } catch {
    return false;
  }
}

/** A page's own interval under the chosen setting, or `null` when refresh is off. */
export function scaleRefreshInterval(baseMilliseconds: number, preference: RefreshPreference): number | null {
  const chosen = REFRESH_OPTIONS.find((option) => option.value === preference)?.milliseconds;
  if (chosen === null) return null;
  const scaled = Math.round((baseMilliseconds * (chosen ?? BASELINE_MILLISECONDS)) / BASELINE_MILLISECONDS);
  return Math.max(Math.min(baseMilliseconds, MIN_REFRESH_MILLISECONDS), scaled);
}
