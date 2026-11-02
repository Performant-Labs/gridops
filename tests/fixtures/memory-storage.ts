/**
 * An in-memory `window.localStorage` for jsdom tests. Recent Node releases
 * ship their own `localStorage` global, which hides jsdom's and is undefined
 * unless Node is started with a storage file, so tests install this instead.
 */
export class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();
  get length() { return this.values.size; }
  clear() { this.values.clear(); }
  getItem(key: string) { return this.values.get(key) ?? null; }
  key(index: number) { return [...this.values.keys()][index] ?? null; }
  removeItem(key: string) { this.values.delete(key); }
  setItem(key: string, value: string) { this.values.set(key, String(value)); }
}

export function installMemoryStorage(): MemoryStorage {
  const storage = new MemoryStorage();
  Object.defineProperty(window, "localStorage", { configurable: true, value: storage });
  return storage;
}
