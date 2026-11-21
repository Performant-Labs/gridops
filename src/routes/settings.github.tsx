import { createFileRoute, redirect } from "@tanstack/react-router";

// The GitHub page moved into Settings → Connections; keep old links working.
export const Route = createFileRoute("/settings/github")({
  beforeLoad: () => {
    throw redirect({ replace: true, to: "/settings/connections" });
  },
});
