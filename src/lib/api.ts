export type Viewer = {
  id: string;
  githubId: number;
  login: string;
  name: string | null;
  email: string | null;
  avatarUrl: string | null;
  role: "admin" | "member";
  alerts: { failedRunners: number; failedWebhooks: number; queuedJobs: number; deferredRunnerCleanup: number };
};

type ApiOptions = Omit<RequestInit, "body"> & { body?: unknown };

/**
 * A non-2xx response; `status` lets callers tell client mistakes from server failures. `code` and
 * `details` are the body's, when it has them: a 422 names what is wrong and where (for example the
 * job category rules whose pattern does not compile, `[{position, message}]`).
 */
export class ApiError extends Error {
  status: number;
  code: string | undefined;
  details: unknown;

  constructor(message: string, status: number, code?: string, details?: unknown) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

export async function api<T>(path: string, options: ApiOptions = {}): Promise<T> {
  const headers = new Headers(options.headers);
  headers.set("Accept", "application/json");
  if (options.body !== undefined) headers.set("Content-Type", "application/json");
  const response = await fetch(path, {
    ...options,
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
    credentials: "same-origin",
    headers,
  });
  if (!response.ok) {
    const error = await response.json().catch(() => null) as { error?: string; code?: unknown; details?: unknown } | null;
    throw new ApiError(
      error?.error ?? `GridOps request failed (${response.status}).`,
      response.status,
      typeof error?.code === "string" ? error.code : undefined,
      error?.details,
    );
  }
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

export async function getViewer(): Promise<Viewer | null> {
  return api<Viewer | null>("/api/v1/auth/session");
}
