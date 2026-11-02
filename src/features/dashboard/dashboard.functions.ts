import type { CapacityHistory, CapacityWindow, DashboardOverview, QueuedJobsPage } from "./types";
import { api } from "~/lib/api";

export function getDashboardOverview() {
  return api<DashboardOverview>("/api/v1/overview");
}

export function getCapacityHistory(window: CapacityWindow) {
  return api<CapacityHistory>(`/api/v1/overview/capacity?window=${window}`);
}

export function getQueuedJobsPage({ page = 1, perPage = 25 }: { page?: number; perPage?: number } = {}) {
  return api<QueuedJobsPage>(`/api/v1/overview/queued-jobs?page=${page}&perPage=${perPage}`);
}
