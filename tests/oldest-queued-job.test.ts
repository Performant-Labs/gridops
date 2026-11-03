// @vitest-environment jsdom
/** The Overview's Oldest queued tile names the job that has waited longest and when GitHub last reported it. */
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { OldestQueuedJob, jobConfirmation } from "../src/features/dashboard/queued-job-reference";
import type { QueuedJobReference } from "../src/features/dashboard/types";
import { formatDateTime } from "../src/lib/utils";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

const job: QueuedJobReference = {
  id: 9001,
  name: "test (macos)",
  repository: "octo/app",
  workflow: "Release",
  htmlUrl: "https://github.com/octo/app/actions/runs/77/job/9001",
  status: "waiting",
  queuedAt: "2026-11-03T15:00:00.000Z",
  confirmedAt: "2026-11-03T15:30:00.000Z",
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("oldest queued job", () => {
  it("says what GitHub last reported, or that it has not yet", () => {
    expect(jobConfirmation(job, (iso) => `<${iso}>`)).toBe("GitHub last reported it waiting at <2026-11-03T15:30:00.000Z>.");
    expect(jobConfirmation({ ...job, confirmedAt: null }, () => "never")).toBe("Not yet confirmed with GitHub.");
  });

  it("names the job with a link to it on GitHub and when GitHub last reported it", () => {
    act(() => root.render(createElement(OldestQueuedJob, { job })));
    const link = container.querySelector("a");
    expect(link?.getAttribute("href")).toBe(job.htmlUrl);
    expect(link?.getAttribute("target")).toBe("_blank");
    expect(link?.textContent).toBe("test (macos) · octo/app · Release");
    expect(container.textContent).toContain(`GitHub last reported it waiting at ${formatDateTime(job.confirmedAt ?? "")}.`);
  });

  it("shows nothing when the queue is clear or an older server names no job", () => {
    act(() => root.render(createElement(OldestQueuedJob, { job: null })));
    expect(container.innerHTML).toBe("");
    act(() => root.render(createElement(OldestQueuedJob, { job: undefined })));
    expect(container.innerHTML).toBe("");
  });
});
