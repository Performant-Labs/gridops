import { describe, expect, it } from "vitest";

import {
  DEFAULT_REFRESH_PREFERENCE,
  MIN_REFRESH_MILLISECONDS,
  REFRESH_OPTIONS,
  REFRESH_STORAGE_KEY,
  readRefreshPreference,
  resolveRefreshPreference,
  scaleRefreshInterval,
  writeRefreshPreference,
} from "~/lib/refresh-interval";

describe("refresh preference", () => {
  it("offers 5 seconds, 10 seconds, 30 seconds, 1 minute and Off, defaulting to 10 seconds", () => {
    expect(REFRESH_OPTIONS.map((option) => option.label)).toEqual(["5 seconds", "10 seconds", "30 seconds", "1 minute", "Off"]);
    expect(DEFAULT_REFRESH_PREFERENCE).toBe("10s");
  });

  it("uses a GridOps-specific storage key", () => {
    expect(REFRESH_STORAGE_KEY).toBe("gridops-refresh-interval");
  });

  it("restores every stored choice", () => {
    for (const option of REFRESH_OPTIONS) {
      expect(readRefreshPreference({ getItem: () => option.value })).toBe(option.value);
    }
  });

  it("falls back to the default for a missing or unknown stored value", () => {
    expect(resolveRefreshPreference(null)).toBe("10s");
    expect(resolveRefreshPreference(undefined)).toBe("10s");
    expect(resolveRefreshPreference("2s")).toBe("10s");
    expect(resolveRefreshPreference("10000")).toBe("10s");
    expect(resolveRefreshPreference("OFF")).toBe("10s");
    expect(readRefreshPreference({ getItem: () => "garbage" })).toBe("10s");
  });

  it("falls back to the default when browser storage is unavailable", () => {
    expect(readRefreshPreference(null)).toBe("10s");
    expect(readRefreshPreference({ getItem: () => { throw new Error("blocked"); } })).toBe("10s");
  });

  it("writes the choice under its key and survives a storage that refuses writes", () => {
    const written: Array<[string, string]> = [];
    expect(writeRefreshPreference({ setItem: (key, value) => { written.push([key, value]); } }, "off")).toBe(true);
    expect(written).toEqual([["gridops-refresh-interval", "off"]]);
    expect(writeRefreshPreference({ setItem: () => { throw new Error("quota"); } }, "30s")).toBe(false);
    expect(writeRefreshPreference(null, "30s")).toBe(false);
  });
});

describe("scaled refresh interval", () => {
  it("keeps every page's current interval at the default", () => {
    for (const base of [3_000, 5_000, 10_000, 30_000, 60_000]) expect(scaleRefreshInterval(base, "10s")).toBe(base);
  });

  it("scales each page's interval in proportion to the chosen one", () => {
    expect(scaleRefreshInterval(10_000, "5s")).toBe(5_000);
    expect(scaleRefreshInterval(10_000, "30s")).toBe(30_000);
    expect(scaleRefreshInterval(10_000, "1m")).toBe(60_000);
    expect(scaleRefreshInterval(5_000, "30s")).toBe(15_000);
    expect(scaleRefreshInterval(3_000, "1m")).toBe(18_000);
    expect(scaleRefreshInterval(30_000, "5s")).toBe(15_000);
  });

  it("never polls faster than the fastest interval GridOps uses today", () => {
    expect(MIN_REFRESH_MILLISECONDS).toBe(3_000);
    expect(scaleRefreshInterval(5_000, "5s")).toBe(3_000);
    expect(scaleRefreshInterval(3_000, "5s")).toBe(3_000);
  });

  it("keeps an active run view at least as fast as the chosen interval", () => {
    for (const option of REFRESH_OPTIONS) {
      if (option.milliseconds === null) continue;
      expect(scaleRefreshInterval(3_000, option.value)).toBeLessThanOrEqual(option.milliseconds);
    }
  });

  it("returns no interval when refresh is off", () => {
    expect(scaleRefreshInterval(10_000, "off")).toBeNull();
    expect(scaleRefreshInterval(3_000, "off")).toBeNull();
  });
});
