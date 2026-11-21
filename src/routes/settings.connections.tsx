import { createFileRoute, getRouteApi } from "@tanstack/react-router";
import { Github } from "lucide-react";

import { Callout, EmptyState } from "~/components/page";
import { SettingsLayout } from "~/components/settings-ui";
import { ConnectionsSection } from "~/features/platform-connections/connections-section";
import { GitHubAppSection } from "~/features/platform-connections/github-app-section";

export const Route = createFileRoute("/settings/connections")({
  component: ConnectionsSettings,
});

const settingsRoute = getRouteApi("/settings");

/** Settings → Connections: every CI platform's connections in one list, plus the GitHub App setup. */
function ConnectionsSettings() {
  const page = settingsRoute.useLoaderData();
  const isAdmin = page.data?.user.role === "admin";

  return (
    <SettingsLayout
      description="Connect each CI platform once, then assign its repositories, runner groups or workspaces to runner pools. Secrets are never shown here."
      title="Connections"
    >
      {/* The one read-only notice for members; the sections below show none of their own. */}
      {page.data && !isAdmin ? <Callout title="Read-only access">A GridOps administrator adds or changes platform connections and manages the GitHub App.</Callout> : null}
      <ConnectionsSection page={page} />
      {page.data ? (
        <GitHubAppSection configuration={page.data.configuration} githubApp={page.data.githubApp} user={page.data.user} />
      ) : <EmptyState className="min-h-0" description="Sign in to manage the GitHub integration." icon={Github} title="GitHub unavailable" />}
    </SettingsLayout>
  );
}
