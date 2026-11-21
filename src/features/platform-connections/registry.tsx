/**
 * The frontend platform registry (#145). Every platform-aware surface reads this list; adding a CI
 * platform means adding its descriptor module and one entry here.
 */
import { CircleHelp } from "lucide-react";

import { bitbucketPlatform } from "./bitbucket";
import { githubPlatform } from "./github";
import type { PlatformDescriptor } from "./types";

export const platforms: readonly PlatformDescriptor[] = [githubPlatform, bitbucketPlatform];

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
