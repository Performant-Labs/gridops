/**
 * The frontend platform registry (#145). Every platform-aware surface reads this list; adding a CI
 * platform means adding its descriptor module and one entry here.
 */
import { CircleHelp } from "lucide-react";

import { bitbucketPlatform } from "./bitbucket";
import { githubPlatform } from "./github";
import { gitlabPlatform } from "./gitlab";
import type { PlatformDescriptor } from "./types";

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
