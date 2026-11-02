export type DashboardOverview = {
  authenticated: boolean;
  configuration: {
    githubOAuth: boolean;
    githubAppControl: boolean;
    webhookActive: boolean;
    webhookVerification: boolean;
    secureStorage: boolean;
    runnerManager: boolean;
    callbackUrl: string;
    webhookUrl: string;
  };
  metrics: {
    runners: number;
    online: number;
    busy: number;
    queuedJobs: number;
    successRate: number | null;
  };
  slo: {
    windowHours: number;
    queue: { oldestSeconds: number | null; p95Seconds: number | null };
    startLatency: { sampleSize: number; p50Seconds: number | null; p95Seconds: number | null };
    /** Fleet runner starts by phase; older servers omit it. */
    runnerStarts?: {
      phases: Array<{ id: RunnerStartPhase; sampleSize: number; p50Seconds: number | null; p95Seconds: number | null }>;
      total: { sampleSize: number; p50Seconds: number | null; p95Seconds: number | null };
    } | null;
    /** How long reconciler passes take; older servers omit it. */
    reconcilerPass?: { sampleSize: number; p50Seconds: number | null; p95Seconds: number | null; intervalSeconds: number } | null;
    failures: Array<{ reason: string; count: number }>;
    alerts: Array<{ level: "warning" | "error"; title: string; detail: string; href: "/queued-jobs" | "/workflow-runs" | "/runner-pools" | "/webhooks" }>;
  };
  pools: Array<{
    id: string;
    name: string;
    scope: string;
    desired: number;
    online: number;
    busy: number;
    queue: number;
    mode: string;
    status: string;
  }>;
  runs: Array<{
    id: number;
    repository: string;
    workflow: string;
    branch: string | null;
    status: string;
    conclusion: string | null;
    waitingJobs: number;
    startedAt: string | null;
    completedAt: string | null;
    htmlUrl: string;
  }>;
  /** How each fleet machine ran the viewer's jobs; null when signed out. */
  machines: {
    windowDays: number;
    items: MachinePerformance[];
  } | null;
  activity: Array<{
    id: string;
    level: string;
    event: string;
    message: string;
    runnerId: string | null;
    poolId: string | null;
    createdAt: string;
  }>;
  installations: number;
};

export type MachinePerformance = {
  id: string;
  name: string;
  os: string;
  architecture: string;
  jobs: number;
  succeeded: number;
  failed: number;
  medianSeconds: number | null;
  /** Its time on jobs that also ran elsewhere, as a percentage of their usual time; null until it has enough of them. */
  relativePercent: number | null;
  comparedJobs: number;
  fastest: boolean;
};

export type CapacityWindow = "24h" | "7d" | "30d";

export type CapacityHistory = {
  window: CapacityWindow;
  points: Array<{
    recordedAt: string;
    available: number;
    busy: number;
    queued: number;
  }>;
};

export type RunnerStartPhase = "room" | "preparing" | "waitingToStart" | "starting";

/**
 * Why a queued job is still waiting, from what the scaler and the pool's runners say.
 * Newer servers may send other values; show those as plain text.
 */
export type QueuedJobWaitingReason =
  | "no_matching_pool"
  | "pool_at_capacity"
  | "runner_starting"
  | "waiting_for_runner"
  | "pool_not_visible";

/** One job counted in the Overview's queued total. */
export type QueuedJob = {
  id: number;
  name: string;
  repository: string;
  workflow: string;
  runId: number;
  runUrl: string;
  htmlUrl: string;
  labels: string[];
  /** The pool the scaler counts the job for; null when no pool can run it (or you can't see the pool, see `waitingReason`). */
  matchedPool: { id: string; name: string } | null;
  waitingReason: QueuedJobWaitingReason | (string & {});
  queuedAt: string;
  ageSeconds: number;
};

/** Queued jobs, the longest-waiting first. */
export type QueuedJobsPage = {
  authenticated: boolean;
  items: QueuedJob[];
  total: number;
  page: number;
  perPage: number;
};
