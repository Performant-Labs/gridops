import { z } from "zod";

const configurationShape = {
  name: z
    .string()
    .trim()
    .min(2)
    .max(48)
    .regex(/^[a-z0-9][a-z0-9-]*[a-z0-9]$/, "Use lowercase letters, numbers, and hyphens."),
  mode: z.enum(["ephemeral", "persistent"]),
  provider: z.enum(["docker", "tart"]),
  providers: z.array(z.enum(["docker", "tart"])).min(1).max(2),
  labels: z.array(z.string().trim().min(1).max(64).refine(
    (label) => !["self-hosted", "linux", "macos", "windows", "x64", "arm64", "arm", "x86"].includes(label.toLowerCase()),
    "System labels are assigned automatically by the selected provider.",
  )).max(20),
  // Each image is required only for a provider the pool uses; see validateProvider.
  image: z.string().trim().max(300),
  dockerImage: z.string().trim().max(300),
  tartImage: z.string().trim().max(300),
  macosRuntime: z.enum(["vm", "native"]).default("vm"),
  desiredCount: z.number().int().min(0).max(100),
  minCount: z.number().int().min(0).max(100),
  maxCount: z.number().int().min(1).max(100),
  autoscalingEnabled: z.boolean().default(true),
  queueScaleFactor: z.number().int().min(1).max(20).default(1),
  idleTimeoutMinutes: z.number().int().min(1).max(1_440).default(5),
  cpuLimit: z.number().positive(),
  memoryLimitMb: z.number().int().positive(),
  runnerGroupId: z.number().int().positive().default(1),
} as const;

const repositoryIds = z.array(z.number().int().positive()).max(1_000);
const bitbucketConnectionIds = z.array(z.string().uuid()).max(100);
const gitlabConnectionIds = z.array(z.string().uuid()).max(100);

function validateCapacity(
  value: { desiredCount: number; minCount: number; maxCount: number },
  context: z.RefinementCtx,
) {
  if (value.minCount > value.maxCount) {
    context.addIssue({
      code: "custom",
      path: ["minCount"],
      message: "Minimum capacity cannot exceed maximum capacity.",
    });
  }
  if (value.desiredCount > value.maxCount) {
    context.addIssue({
      code: "custom",
      path: ["desiredCount"],
      message: "Desired capacity cannot exceed maximum capacity.",
    });
  }
  if (value.desiredCount < value.minCount) {
    context.addIssue({
      code: "custom",
      path: ["desiredCount"],
      message: "Desired capacity cannot be below minimum capacity.",
    });
  }
}

function validateProvider(
  value: {
    provider: "docker" | "tart"; providers: Array<"docker" | "tart">; mode: "ephemeral" | "persistent"; cpuLimit: number; memoryLimitMb: number; macosRuntime: "vm" | "native";
    image: string; dockerImage: string; tartImage: string;
  },
  context: z.RefinementCtx,
) {
  if (new Set(value.providers).size !== value.providers.length || value.providers[0] !== value.provider) {
    context.addIssue({ code: "custom", path: ["providers"], message: "Choose unique providers with the primary provider first." });
  }
  const includesTart = value.providers.includes("tart");
  if (includesTart && value.mode !== "ephemeral") {
    context.addIssue({ code: "custom", path: ["mode"], message: "Pools that include Tart must be ephemeral." });
  }
  if (includesTart && !Number.isInteger(value.cpuLimit)) {
    context.addIssue({ code: "custom", path: ["cpuLimit"], message: "Tart runners require whole CPU cores." });
  }
  // Native execution lives in the macOS agent, so a pool without that provider
  // would silently never receive a native runner.
  if (value.macosRuntime === "native" && !includesTart) {
    context.addIssue({ code: "custom", path: ["macosRuntime"], message: "Native execution requires the macOS runner provider." });
  }
  // A provider added to a pool brings its own image; none is filled in from a
  // default (issue #103). `image` stands in for the primary provider's, as on
  // the server. Native execution clones no VM, so it needs no base VM.
  const imageOf = (kind: "docker" | "tart", configured: string) => configured || (value.provider === kind ? value.image : "");
  if (value.providers.includes("docker") && !imageOf("docker", value.dockerImage)) {
    context.addIssue({ code: "custom", path: ["dockerImage"], message: "Docker image (dockerImage) is required for a pool that uses Linux · Docker." });
  }
  if (includesTart && value.macosRuntime !== "native" && !imageOf("tart", value.tartImage)) {
    context.addIssue({ code: "custom", path: ["tartImage"], message: "Tart base VM (tartImage) is required for a pool that uses macOS · Tart virtual machines." });
  }
}

export const updateRunnerPoolSchema = z
  .object({
    repositoryIds: repositoryIds.optional(),
    priorityRepositoryIds: repositoryIds.optional(),
    priorityWaitMinutes: z.number().int().min(1).max(1_440).optional(),
    bitbucketConnectionIds: bitbucketConnectionIds.optional(),
    // Replaces a GitLab pool's connection set; never sent for another pool.
    gitlabConnectionIds: gitlabConnectionIds.optional(),
    runUntagged: z.boolean().optional(),
    ...configurationShape,
  })
  .superRefine((value, context) => {
    if (value.repositoryIds?.length === 0) {
      context.addIssue({
        code: "custom",
        path: ["repositoryIds"],
        message: "Choose at least one repository for the pool.",
      });
    }
    if (value.repositoryIds && value.repositoryIds.length > value.maxCount) {
      context.addIssue({
        code: "custom",
        path: ["repositoryIds"],
        message: "Repository count cannot exceed maximum runner capacity.",
      });
    }
    validateBitbucketConnections(value, context);
    validateGitlabConnections(value, context);
    validateCapacity(value, context);
    validateProvider(value, context);
  });

export const createRunnerPoolSchema = z
  .object({
    installationId: z.number().int().positive(),
    repositoryIds: repositoryIds.default([]),
    bitbucketConnectionIds: bitbucketConnectionIds.default([]),
    gitlabConnectionIds: gitlabConnectionIds.default([]),
    // Whether a GitLab pool's runners also take jobs with no tags.
    runUntagged: z.boolean().default(false),
    scope: z.enum(["repository", "organization", "gitlab"]),
    ...configurationShape,
  })
  .superRefine((value, context) => {
    if (value.scope === "repository" && value.repositoryIds.length === 0) {
      context.addIssue({
        code: "custom",
        path: ["repositoryIds"],
        message: "Choose at least one repository for a repository-scoped pool.",
      });
    }
    if (value.scope === "organization" && value.repositoryIds.length > 0) {
      context.addIssue({
        code: "custom",
        path: ["repositoryIds"],
        message: "Organization pools use runner-group repository access.",
      });
    }
    if (value.repositoryIds.length > value.maxCount) {
      context.addIssue({
        code: "custom",
        path: ["repositoryIds"],
        message: "Repository count cannot exceed maximum runner capacity.",
      });
    }
    validateBitbucketConnections(value, context);
    validateGitlabTargets(value, context);
    validateGitlabConnections(value, context);
    validateCapacity(value, context);
    validateProvider(value, context);
  });

export function parseCreateRunnerPoolInput(input: unknown) {
  const result = createRunnerPoolSchema.safeParse(input);
  if (!result.success) {
    throw new Error(result.error.issues[0]?.message ?? "Runner pool configuration is invalid.");
  }
  return result.data;
}

function validateBitbucketConnections(
  value: { bitbucketConnectionIds?: string[]; providers: Array<"docker" | "tart">; labels: string[]; maxCount: number },
  context: z.RefinementCtx,
) {
  const connections = value.bitbucketConnectionIds;
  if (!connections?.length) return;
  if (!value.providers.includes("tart")) {
    context.addIssue({ code: "custom", path: ["bitbucketConnectionIds"], message: "Bitbucket macOS runners require Tart in this first adapter release." });
  }
  if (connections.length > value.maxCount) {
    context.addIssue({ code: "custom", path: ["bitbucketConnectionIds"], message: "Bitbucket workspace count cannot exceed maximum runner capacity." });
  }
  const labelError = bitbucketLabelError(value.labels);
  if (labelError) context.addIssue({ code: "custom", path: ["labels"], message: labelError });
}

/**
 * The create-only GitLab scope rule (the server's `validate_gitlab_targets`): a pool is GitLab-scoped
 * exactly when it has a GitLab connection, and a GitLab pool has no GitHub repositories and no
 * Bitbucket workspaces. The update schema has no scope, so it cannot apply this rule.
 */
function validateGitlabTargets(
  value: { scope: "repository" | "organization" | "gitlab"; repositoryIds: number[]; bitbucketConnectionIds: string[]; gitlabConnectionIds: string[] },
  context: z.RefinementCtx,
) {
  if (value.scope !== "gitlab") {
    if (value.gitlabConnectionIds.length > 0) {
      context.addIssue({ code: "custom", path: ["gitlabConnectionIds"], message: "Only a GitLab pool can use GitLab projects or groups." });
    }
    return;
  }
  if (value.gitlabConnectionIds.length === 0) {
    context.addIssue({ code: "custom", path: ["gitlabConnectionIds"], message: "Choose at least one GitLab project or group for a GitLab pool." });
  }
  if (value.repositoryIds.length > 0) {
    context.addIssue({ code: "custom", path: ["repositoryIds"], message: "A GitLab pool cannot also use GitHub repositories." });
  }
  if (value.bitbucketConnectionIds.length > 0) {
    context.addIssue({ code: "custom", path: ["bitbucketConnectionIds"], message: "A GitLab pool cannot also use Bitbucket workspaces." });
  }
}

/**
 * The GitLab connection bounds shared by create and update (the server's `validate_gitlab_connections`):
 * unique, and no more than the maximum runner count. Their labels are the runners' GitLab tags.
 */
function validateGitlabConnections(
  value: { gitlabConnectionIds?: string[]; labels: string[]; maxCount: number },
  context: z.RefinementCtx,
) {
  const connections = value.gitlabConnectionIds;
  if (!connections?.length) return;
  if (new Set(connections).size !== connections.length) {
    context.addIssue({ code: "custom", path: ["gitlabConnectionIds"], message: "Choose each GitLab project or group once." });
  }
  if (connections.length > value.maxCount) {
    context.addIssue({ code: "custom", path: ["gitlabConnectionIds"], message: "GitLab project and group count cannot exceed maximum runner capacity." });
  }
  const labelError = gitlabLabelError(value.labels);
  if (labelError) context.addIssue({ code: "custom", path: ["labels"], message: labelError });
}

/**
 * The GitLab tag rule, shared by the submit-time schema and the GitLab platform descriptor: any
 * character but a comma, and not blank. The shared label schema above caps every label at 64.
 */
export function gitlabLabelError(labels: readonly string[]): string | null {
  return labels.some((label) => !label.trim() || label.includes(","))
    ? "GitLab tags cannot be blank or contain a comma."
    : null;
}

/** The Bitbucket label rule, shared by the submit-time schema and the Bitbucket platform descriptor. */
export function bitbucketLabelError(labels: readonly string[]): string | null {
  return labels.some((label) => !/^[a-z0-9.]+$/.test(label))
    ? "Bitbucket-compatible labels use lowercase letters, numbers, and dots."
    : null;
}

export function parseUpdateRunnerPoolInput(input: unknown) {
  const result = updateRunnerPoolSchema.safeParse(input);
  if (!result.success) {
    throw new Error(result.error.issues[0]?.message ?? "Runner pool configuration is invalid.");
  }
  return result.data;
}

export type CreateRunnerPoolInput = z.infer<typeof createRunnerPoolSchema>;
export type UpdateRunnerPoolInput = z.infer<typeof updateRunnerPoolSchema>;
