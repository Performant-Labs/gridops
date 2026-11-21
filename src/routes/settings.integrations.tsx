import { createFileRoute, redirect } from "@tanstack/react-router";

// The Bitbucket page moved into Settings → Connections; keep old links working.
export const Route = createFileRoute("/settings/integrations")({
  beforeLoad: () => {
    throw redirect({ replace: true, to: "/settings/connections" });
  },
});
