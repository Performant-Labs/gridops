import { api } from "~/lib/api";

export type BitbucketConnection = {
  id: string;
  provider: "bitbucket";
  name: string;
  workspace: string;
  workspaceUuid: string;
  createdAt: string;
  updatedAt: string;
  canManage: boolean;
};

export const getBitbucketConnections = (signal?: AbortSignal) =>
  api<{ canManage: boolean; items: BitbucketConnection[] }>("/api/v1/platform-connections/bitbucket", { signal });

export const createBitbucketConnection = (data: { name: string; workspace: string; accessToken: string }) =>
  api<Pick<BitbucketConnection, "id" | "provider" | "name" | "workspace" | "workspaceUuid">>(
    "/api/v1/platform-connections/bitbucket",
    { method: "POST", body: data },
  );

/** Renames a connection and, when `accessToken` is set, replaces its token. */
export const updateBitbucketConnection = (connectionId: string, data: { name: string; accessToken?: string }) =>
  api<BitbucketConnection>(`/api/v1/platform-connections/bitbucket/${encodeURIComponent(connectionId)}`, { method: "PUT", body: data });

export const removeBitbucketConnection = (connectionId: string) =>
  api<void>(`/api/v1/platform-connections/bitbucket/${encodeURIComponent(connectionId)}`, { method: "DELETE" });

/** A connected GitLab project or group (docs/openapi/gridops-v1.json `GitlabConnection`). Never carries the token or webhook secret. */
export type GitlabConnection = {
  id: string;
  provider: "gitlab";
  name: string;
  baseUrl: string;
  scopeKind: "project" | "group";
  scopeId: number;
  scopePath: string;
  createdAt: string;
  updatedAt: string;
  canManage: boolean;
};

/** A GitLab connection's webhook address and secret, returned once at create and at each rotation. */
export type GitlabWebhookSecret = { webhookUrl: string; webhookSecret: string };

export type GitlabConnectionCreated = GitlabWebhookSecret & { connection: GitlabConnection };

const GITLAB_CONNECTIONS = "/api/v1/platform-connections/gitlab";
const gitlabConnectionPath = (connectionId: string) => `${GITLAB_CONNECTIONS}/${encodeURIComponent(connectionId)}`;

export const getGitlabConnections = (signal?: AbortSignal) =>
  api<{ canManage: boolean; items: GitlabConnection[] }>(GITLAB_CONNECTIONS, { signal });

/** Verifies the token and scope, then stores the connection; resolves to the one-time webhook secret. */
export const createGitlabConnection = (data: { name: string; baseUrl?: string; scopeKind: "project" | "group"; scopePath: string; accessToken: string }) =>
  api<GitlabConnectionCreated>(GITLAB_CONNECTIONS, { method: "POST", body: data });

/** Renames a connection and, when `accessToken` is set, replaces its token (a partial PATCH). */
export const updateGitlabConnection = (connectionId: string, data: { name?: string; accessToken?: string }) =>
  api<GitlabConnection>(gitlabConnectionPath(connectionId), { method: "PATCH", body: data });

export const removeGitlabConnection = (connectionId: string) =>
  api<void>(gitlabConnectionPath(connectionId), { method: "DELETE" });

/** Checks the stored token still has its scopes and role; rejects with the server's reason. */
export const verifyGitlabConnection = (connectionId: string) =>
  api<void>(`${gitlabConnectionPath(connectionId)}/verify`, { method: "POST" });

/** Replaces the webhook secret; the old one stops working at once. */
export const rotateGitlabWebhookSecret = (connectionId: string) =>
  api<GitlabWebhookSecret>(`${gitlabConnectionPath(connectionId)}/webhook-secret/rotate`, { method: "POST" });
