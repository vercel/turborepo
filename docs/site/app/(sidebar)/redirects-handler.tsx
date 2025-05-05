"use client";

import { usePathname, useRouter } from "next/navigation";
import { useEffect } from "react";

// NOTE: (May 2024) This component handles redirects that *start at hashlinks*.
// This can only be done on the client, since the server cannot see the hash.
const clientRedirectsMap = {
  "/docs": {
    "#monorepos": { hash: "the-monorepo-problem" },
    "#examples": {
      location: "/docs/getting-started/installation",
      hash: "start-with-an-example",
    },
    "#should-i-install-turborepo-globally": {
      location: "/docs/getting-started/installation",
      hash: "global-installation",
    },
    "#does-turborepo-collect-any-personally-identifiable-information-when-using-remote-caching":
      {
        location: "/docs/telemetry",
      },
    "#can-i-use-turborepo-with-a-monorepo-that-non-js-code": {
      location: "/docs/guides/multi-language",
    },
    "#why-isnt-my-global-turbo-working-as-expected": {
      location: "/docs/getting-started/installation",
      hash: "global-installation",
    },
    "#do-i-have-to-use-remote-caching-to-use-turborepo": {
      location: "/docs/core-concepts/remote-caching",
    },
    "#do-i-have-to-use-vercel-to-use-turborepo": {
      location: "/docs/core-concepts/remote-caching",
    },
    "#does-turborepo-collect-any-personally-identifiable-information": {
      location: "/docs/telemetry",
    },
    "#does-turborepo--remote-caching-store-my-source-code": {
      location: "/docs/crafting-your-repository/caching",
      hash: "what-gets-cached",
    },
    "#can-i-use-turborepo-with-a-different-remote-cache-provider-other-than-vercel":
      {
        location: "/docs/core-concepts/remote-caching",
      },
    "#how-can-i-retain-fast-refresh-in-my-turborepo-when-using-multiple-nextjs-applications":
      {
        location: "/docs/core-concepts/internal-packages",
        hash: "just-in-time-packages",
      },
    "#what-does-experimental-mean": {
      location: "/governance",
      hash: "#stability-and-release-phases",
    },
  },
  "/docs/getting-started/installation": {
    "#install-globally": { hash: "global-installation" },
    "#install-per-repository": { hash: "repository-installation" },
  },
  "/docs/crafting-your-repository/caching": {
    "#handling-nodejs-versions": {
      location: "/docs/guides/handling-platforms",
    },
    "#hashing": {
      hash: "task-inputs",
    },
    "#handling-platforms-and-other-arbitrary-hash-contributors": {
      location: "/docs/guides/handling-platforms",
      hash: "#operating-systems-architecture-and-other-arbitrary-conditions",
    },
    "#1-write-an-arbitrary-file-to-disk": {
      location: "/docs/guides/handling-platforms",
      hash: "1-write-an-arbitrary-file-to-disk",
    },
    "#2-add-the-file-to-your-gitignore": {
      location: "/docs/guides/handling-platforms",
      hash: "#2-add-the-file-to-your-gitignore",
    },
    "#3-add-the-file-to-the-hash": {
      location: "/docs/guides/handling-platforms",
      hash: "#3-add-the-file-to-the-hash",
    },
    "#4-generate-the-file-before-running-turbo": {
      location: "/docs/guides/handling-platforms",
      hash: "#4-generate-the-file-before-running-turbo",
    },
  },
  "/docs/crafting-your-repository/running-tasks": {
    "#most-tools-dont-optimize-for-speed": {
      location: "/docs/crafting-your-repository/configuring-tasks",
    },
    "#turborepo-can-multitask": {
      location: "/docs/crafting-your-repository/configuring-tasks",
    },
    "#defining-a-pipeline": {
      location: "/docs/crafting-your-repository/configuring-tasks",
    },
    "#running-tasks-from-the-root": {
      location: "/docs/crafting-your-repository/configuring-tasks",
      hash: "registering-root-tasks",
    },
    "#incremental-adoption": {
      location: "/docs/crafting-your-repository/configuring-tasks",
      hash: "when-to-use-root-tasks",
    },
    "#filtering-by-package-name": {
      location: "/docs/crafting-your-repository/running-tasks",
      hash: "filtering-by-package",
    },
  },
  "/docs/crafting-your-repository/using-environment-variables": {
    "#globalenv": { hash: "adding-environment-variables-to-task-hashes" },
    "#pipelinetaskenv": { hash: "adding-environment-variables-to-task-hashes" },
    "#loose--strict-environment-modes": {
      hash: "environment-modes",
    },
    "#wildcards": {
      location: "/docs/reference/configuration",
      hash: "wildcards",
    },
    "#syntax": {
      location: "/docs/reference/configuration",
      hash: "wildcards",
    },
    "#system-environment-variables": {
      location: "/docs/reference/system-environment-variables",
    },
    "#hashed-environment-variables": {
      hash: "strict-mode",
    },
    "#unhashed-environment-variables": {
      hash: "strict-mode",
    },
    "#infer-mode": {
      hash: "strict-mode",
    },
    "#env-files": {
      hash: "handling-env-files",
    },
    "#framework-inference-exclusions": {
      hash: "framework-inference",
    },
    "#framework-inference-is-per-workspace": {
      hash: "framework-inference",
    },
    "#eslint-config-turbo": {
      hash: "use-eslint-config-turbo",
    },
    "#invisible-environment-variables": {
      hash: "avoid-creating-or-mutating-environment-variables-at-runtime",
    },
  },
  "/docs/crafting-your-repository/configuring-tasks": {
    "#from-the-same-workspace": {
      hash: "depending-on-tasks-in-the-same-package",
    },
    "#from-dependent-workspaces": {
      hash: "#depending-on-tasks-in-dependencies-with-",
    },
    "#from-arbitrary-workspaces": {
      hash: "depending-on-a-specific-task-in-a-specific-package",
    },
    "#dependencies-outside-of-a-task": {
      hash: "dependent-tasks-that-can-be-ran-in-parallel",
    },
  },
  "/docs/reference/configuration": {
    "#glob-specification-for-paths": {
      location: "/docs/reference/globs",
    },
  },
  "/docs/reference": {
    "#global-arguments": {
      hash: "global-flags",
    },
  },
  "/docs/core-concepts/internal-packages": {
    "#anatomy-of-a-package": {
      location: "/docs/crafting-your-repository/structuring-a-repository",
      hash: "anatomy-of-a-package",
    },
  },
};

const handleRedirect = (
  router: ReturnType<typeof useRouter>,
  pathname: string | null
): void => {
  const redirectList:
    | Record<string, { hash?: string; location?: string }>
    | undefined =
    clientRedirectsMap[pathname as keyof typeof clientRedirectsMap];

  const newHash: string | undefined =
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- These are correct.
    redirectList?.[window.location.hash]?.hash;

  const newLocation: string | undefined =
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- These are correct.
    redirectList?.[window.location.hash]?.location;

  if (newHash && newLocation) {
    /* eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- This is needed or else we can get crashes.
     * */
    router.push(`${newLocation ?? ""}${newHash ? `#${newHash}` : ""}`);
    return;
  }

  if (newHash) {
    window.location.hash = newHash;
    return;
  }

  if (newLocation) {
    router.push(newLocation);
  }
};

export function RedirectsHandler(): null {
  const pathname = usePathname();
  const router = useRouter();

  useEffect(() => {
    handleRedirect(router, pathname);
  }, []);

  return null;
}
