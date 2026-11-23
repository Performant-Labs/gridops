import { createFileRoute } from "@tanstack/react-router";

import { FleetHealthPage } from "~/features/fleet-health/page";

export const Route = createFileRoute("/_app/fleet/health")({
  component: FleetHealthPage,
});
