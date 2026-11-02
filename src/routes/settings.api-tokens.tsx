import { createFileRoute, getRouteApi } from "@tanstack/react-router";
import { KeyRound } from "lucide-react";

import { Callout, EmptyState } from "~/components/page";
import { SettingsLayout } from "~/components/settings-ui";
import { CreateTokenSection, TokenListSection } from "~/features/fleet/api-tokens-section";

export const Route = createFileRoute("/settings/api-tokens")({
  component: ApiTokensSettings,
});

const settingsRoute = getRouteApi("/settings");

function ApiTokensSettings() {
  const page = settingsRoute.useLoaderData();
  if (!page.data) return <EmptyState description="Sign in to manage API tokens." icon={KeyRound} title="API tokens unavailable" />;
  const isAdmin = page.data.user.role === "admin";

  return (
    <SettingsLayout
      description="Scoped bearer tokens let scripts, CI jobs and bots call the fleet API without a browser session. A token acts as you, with your current role, limited to the scopes you give it."
      title="API tokens"
    >
      {isAdmin ? (
        <>
          <CreateTokenSection />
          <TokenListSection />
        </>
      ) : (
        <Callout title="Administrators only">
          Only a GridOps administrator can create, list and revoke API tokens. Ask an administrator for a token, or for administrator access.
        </Callout>
      )}
    </SettingsLayout>
  );
}
