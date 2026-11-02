import { useSyncExternalStore } from "react";

import {
  readRefreshPreference,
  REFRESH_STORAGE_KEY,
  resolveRefreshPreference,
  scaleRefreshInterval,
  writeRefreshPreference,
  type RefreshPreference,
} from "~/lib/refresh-interval";

/** `window.localStorage` itself throws in some privacy modes; treat that as no storage. */
function browserStorage(): Storage | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}

// One shared value for every live page, held in memory so the choice still
// applies for this page view when storage refuses it.
let current: RefreshPreference | undefined;
const preferenceListeners = new Set<() => void>();

export function getRefreshPreference(): RefreshPreference {
  current ??= readRefreshPreference(browserStorage());
  return current;
}

export function setRefreshPreference(preference: RefreshPreference) {
  current = preference;
  writeRefreshPreference(browserStorage(), preference);
  for (const listener of preferenceListeners) listener();
}

/** Forgets the in-memory choice so the next read comes from storage again. */
export function resetRefreshPreferenceStore() {
  current = undefined;
  for (const listener of preferenceListeners) listener();
}

function handleStorage(event: StorageEvent) {
  // Another tab changed the setting: follow it here too.
  if (event.key !== REFRESH_STORAGE_KEY) return;
  current = resolveRefreshPreference(event.newValue);
  for (const listener of preferenceListeners) listener();
}

function subscribePreference(listener: () => void) {
  if (preferenceListeners.size === 0 && typeof window !== "undefined") window.addEventListener("storage", handleStorage);
  preferenceListeners.add(listener);
  return () => {
    preferenceListeners.delete(listener);
    if (preferenceListeners.size === 0 && typeof window !== "undefined") window.removeEventListener("storage", handleStorage);
  };
}

export function useRefreshPreference(): [RefreshPreference, (preference: RefreshPreference) => void] {
  const preference = useSyncExternalStore(subscribePreference, getRefreshPreference, getRefreshPreference);
  return [preference, setRefreshPreference];
}

/**
 * A live view's own interval under the chosen setting, or `null` when refresh
 * is off. Pass the interval the view uses at the default (10 seconds) setting.
 */
export function useRefreshInterval(baseMilliseconds: number): number | null {
  const [preference] = useRefreshPreference();
  return scaleRefreshInterval(baseMilliseconds, preference);
}

// A counter bumped by a manual Refresh, so sections that load their own data
// (charts, activity lists) reload alongside the page.
let refreshSignal = 0;
const signalListeners = new Set<() => void>();

export function signalRefresh() {
  refreshSignal += 1;
  for (const listener of signalListeners) listener();
}

function subscribeSignal(listener: () => void) {
  signalListeners.add(listener);
  return () => void signalListeners.delete(listener);
}

const getSignal = () => refreshSignal;

/** Changes whenever someone presses Refresh; add it to a loading effect's dependencies. */
export function useRefreshSignal(): number {
  return useSyncExternalStore(subscribeSignal, getSignal, getSignal);
}
