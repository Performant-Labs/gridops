import { describe, expect, it } from "vitest";
import { allNavItems } from "~/lib/navigation";

describe("sidebar navigation", () => {
  it("links the Queued jobs page and carries the queued-jobs count there", () => {
    const item = allNavItems.find((entry) => entry.to === "/queued-jobs");
    expect(item?.label).toBe("Queued jobs");
    expect(item?.alert).toBe("queuedJobs");
  });

  it("shows the queued-jobs count on exactly one item", () => {
    expect(allNavItems.filter((entry) => entry.alert === "queuedJobs")).toHaveLength(1);
  });

  it("gives every item its own destination and its own jump shortcut", () => {
    const destinations = allNavItems.map((entry) => entry.to);
    const shortcuts = allNavItems.map((entry) => entry.shortcut);
    expect(new Set(destinations).size).toBe(destinations.length);
    expect(new Set(shortcuts).size).toBe(shortcuts.length);
  });
});
