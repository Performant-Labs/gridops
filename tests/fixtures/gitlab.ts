/**
 * GitLab fixtures for the #141 tests: connections shaped as `GitlabConnection` in
 * docs/openapi/gridops-v1.json (from #137), the pool-side `{ id, name, baseUrl, scopeKind, scopePath }`
 * projection the pool options and pool detail carry (#138), and a fetch stub for the GitLab
 * connection routes.
 */
import { createOptions, poolFormFetch, settingsPool } from "./pool-forms";

export const GITLAB_URL = "/api/v1/platform-connections/gitlab";

export const GITLAB_CONNECTIONS = [
  {
    id: "a1000000-0000-4000-8000-000000000001", provider: "gitlab", name: "Mobile app", baseUrl: "https://gitlab.com",
    scopeKind: "project", scopeId: 101, scopePath: "acme/platform/mobile-app",
    createdAt: "2026-11-01T20:00:00.000Z", updatedAt: "2026-11-20T18:00:00.000Z", canManage: true,
  },
  {
    id: "a1000000-0000-4000-8000-000000000002", provider: "gitlab", name: "Platform group", baseUrl: "https://git.example.com",
    scopeKind: "group", scopeId: 7, scopePath: "platform",
    createdAt: "2026-11-01T20:00:00.000Z", updatedAt: "2026-11-19T18:00:00.000Z", canManage: true,
  },
  {
    id: "a1000000-0000-4000-8000-000000000003", provider: "gitlab", name: "Docs site", baseUrl: "https://gitlab.com",
    scopeKind: "project", scopeId: 102, scopePath: "acme/docs",
    createdAt: "2026-11-01T20:00:00.000Z", updatedAt: "2026-11-18T18:00:00.000Z", canManage: true,
  },
] as const;

export const [MOBILE_APP, PLATFORM_GROUP, DOCS_SITE] = GITLAB_CONNECTIONS;

/** The `gitlabConnections` entry the pool options and pool detail carry for a connection. */
export const poolConnection = (connection: (typeof GITLAB_CONNECTIONS)[number]) => ({
  id: connection.id, name: connection.name, baseUrl: connection.baseUrl, scopeKind: connection.scopeKind, scopePath: connection.scopePath,
});

/** The new-pool options with GitLab connections (the first two, unless given). */
export function gitlabCreateOptions(connections: ReadonlyArray<(typeof GITLAB_CONNECTIONS)[number]> = [MOBILE_APP, PLATFORM_GROUP], overrides: Parameters<typeof createOptions>[0] = {}) {
  return { ...createOptions(overrides), gitlabConnections: connections.map(poolConnection) };
}

/** A GitLab pool on installation 1 using Mobile app and Platform group. */
export function gitlabPool(overrides: Record<string, unknown> = {}) {
  return settingsPool({
    name: "gitlab-builds", scope: "gitlab", installationId: 1, accountLogin: "octo-org", runnerGroupId: 1,
    gitlabConnections: [poolConnection(MOBILE_APP), poolConnection(PLATFORM_GROUP)],
    gitlabConnectionIds: [MOBILE_APP.id, PLATFORM_GROUP.id],
    runUntagged: false,
    ...overrides,
  });
}

/** `poolFormFetch` plus `GET /api/v1/platform-connections/gitlab` (every connection) and the pool-1 PUT. */
export function gitlabPoolFormFetch(url: string, init?: RequestInit): Response | Promise<Response> {
  const method = init?.method ?? "GET";
  if (method === "GET" && url === GITLAB_URL) return Response.json({ canManage: true, items: GITLAB_CONNECTIONS });
  return poolFormFetch(url, init);
}
