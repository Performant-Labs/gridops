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
    /** Every runner that is not deleted: busy + idle + starting + unavailable. */
    runners: number;
    /** Busy + idle, as before the runner states were split. */
    online: number;
    /** Working on a job (by status, not by the busy flag). */
    busy: number;
    /** Online and not busy: capacity free for the queue. */
    idle: number;
    starting: number;
    /** Paused, stopped, failed or any other state that cannot take a job. */
    unavailable: number;
    /** Every job waiting to start: queuedWaiting + queuedUnmatched. */
    queuedJobs: number;
    /** Queued jobs a pool could run once it has capacity. */
    queuedWaiting: number;
    /** Queued jobs no available pool can ever run (no label match, or the matching pools are paused or tripped). */
    queuedUnmatched: number;
    successRate: number | null;
    /** Days of completed runs the rate covers; older servers omit it. */
    successRateWindowDays?: number;
    /** Runs counted in the rate (skipped and cancelled excluded); older servers omit it. */
    successRateRuns?: number;
  };
  /**
   * How the queue splits over the pool rows: the rows' `queue` plus
   * `otherPools` plus `unmatched` is `metrics.queuedJobs`.
   */
  queue: {
    unmatched: number;
    /** Waiting jobs owned by pools that are not among the listed rows. */
    otherPools: number;
    /** Labels no pool has, with the jobs that ask for them. */
    missingLabels: Array<{ labels: string[]; jobs: number }>;
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
    /** Queued jobs this pool serves; a job is counted by one pool only. */
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
  /**
   * For a job no pool serves: the labels it asked for that no pool you can see offers
   * (a platform label such as `self-hosted-macos` counts as offered only by a pool running
   * that platform). Empty when each label is offered but no available pool offers them all,
   * and for every job a pool serves. Missing from older servers.
   */
  missingLabels?: string[];
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
