/**
 * The frontend platform registry (#145). Every platform-aware surface reads this list; adding a CI
 * platform means adding its descriptor module and one entry here.
 */
import { CircleHelp } from "lucide-react";

import { PropertyRow } from "~/components/page";

import { bitbucketPlatform } from "./bitbucket";
import { githubPlatform } from "./github";
import { gitlabPlatform } from "./gitlab";
import type { PlatformDescriptor, PoolBindings } from "./types";

// Order matters: `buildCreatePayload` merges the create partials in this order, and GitHub's always
// carries its scope, installation, repositories and runner group. A platform whose own pool scope
// replaces those registers after GitHub, so its partial (when it has a target) wins.
export const platforms: readonly PlatformDescriptor[] = [githubPlatform, bitbucketPlatform, gitlabPlatform];

/** A platform this frontend does not know (e.g. one a newer server reports): named by its id, with no targets. */
function unknownPlatform(id: string): PlatformDescriptor {
  return {
    id,
    name: id,
    icon: CircleHelp,
    connections: {
      list: { kind: "page", read: () => ({ canManage: false, items: [] }) },
      describe: (item) => ({ id: String(item), name: String(item) }),
      emptyText: () => "No connections.",
      chooserDescription: "Not supported by this version of GridOps.",
    },
    targetKinds: [],
    toCreatePayload: () => ({}),
    toUpdatePayload: () => ({}),
  };
}

/** The registered descriptor for `id`, or a safe fallback that never throws. */
export function getPlatform(id: string): PlatformDescriptor {
  return platforms.find((platform) => platform.id === id) ?? unknownPlatform(id);
}

/**
 * The platform a webhook delivery came in for. The deliveries list carries no platform field:
 * a platform that stores its deliveries beside GitHub's prefixes their ids with `<platform id>:`.
 * GitHub's delivery GUIDs carry no prefix, so a delivery without a registered one is GitHub's.
 */
export function platformForDeliveryId(deliveryId: string): PlatformDescriptor {
  return platforms.find((platform) => deliveryId.startsWith(`${platform.id}:`)) ?? githubPlatform;
}

/** A platform that owns a pool scope: its pool bindings are always there. */
export type PoolPlatform = PlatformDescriptor & { pool: PoolBindings };

/**
 * The pool destination of a scope no registered platform owns (e.g. one a newer server reports):
 * neutral, with no account, avatar or platform icon. It never shows the raw scope.
 */
function UnknownPoolDestination() {
  return <PropertyRow label="Scope">Unknown</PropertyRow>;
}

const unknownPoolPlatform: PoolPlatform = {
  ...unknownPlatform("unknown"),
  pool: {
    scopes: [],
    destination: () => ({
      text: "—",
      title: "Unknown destination",
      leading: <span aria-hidden="true" className="size-4 shrink-0 rounded-[5px] border border-dashed border-border-strong" />,
    }),
    Destination: UnknownPoolDestination,
    runnerDestination: () => ({ text: "—", title: "Unknown destination" }),
    scopeLabel: () => "Unknown",
  },
};

const hasPoolBindings = (platform: PlatformDescriptor): platform is PoolPlatform => platform.pool !== undefined;

/**
 * The pool platform of a pool whose scope the server did not send (a runner from a server before
 * #169): GitHub's, since every pool was GitHub's until GitLab pools, as `platformForDeliveryId`
 * treats an unprefixed delivery. A scope that is sent but unknown still gets the neutral fallback.
 */
const absentScopePoolPlatform: PoolPlatform = hasPoolBindings(githubPlatform) ? githubPlatform : unknownPoolPlatform;

const ownsPoolScope = (platform: PlatformDescriptor, scope: string): platform is PoolPlatform => platform.pool?.scopes.includes(scope) ?? false;

/**
 * The platform that owns a pool's scope, which says where the pool registers its runners. A
 * platform whose pools ride on another's scope (a Bitbucket workspace on a GitHub pool) owns
 * none, so those pools resolve to the scope's owner. An unknown scope gets a neutral fallback; an
 * absent one (an older server) is GitHub's.
 */
export function platformForPool(pool: { scope?: string | null }): PoolPlatform {
  const { scope } = pool;
  if (scope === undefined || scope === null) return absentScopePoolPlatform;
  return platforms.find((platform): platform is PoolPlatform => ownsPoolScope(platform, scope)) ?? unknownPoolPlatform;
}
