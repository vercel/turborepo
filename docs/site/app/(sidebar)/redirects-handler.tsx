"use client";

import { usePathname, useRouter } from "next/navigation";
import { useEffect } from "react";

// NOTE: (May 2024) This component handles redirects that *start at hashlinks*.
// This can only be done on the client, since the server cannot see the hash.
const clientRedirectsMap = {
  "/repo/docs": {
    "#monorepos": { hash: "the-monorepo-problem" },
    "#examples": {
      location: "/repo/docs/getting-started/installation",
      hash: "start-with-an-example",
    },
    "#should-i-install-turborepo-globally": {
      location: "/repo/docs/getting-started/installation",
      hash: "global-installation",
    },
    "#does-turborepo-collect-any-personally-identifiable-information-when-using-remote-caching":
      {
        location: "/repo/docs/telemetry",
      },
    "#can-i-use-turborepo-with-a-monorepo-that-non-js-code": {
      location: "/repo/docs/guides/multi-language",
    },
    "#why-isnt-my-global-turbo-working-as-expected": {
      location: "/repo/docs/getting-started/installation",
      hash: "global-installation",
    },
    "#do-i-have-to-use-remote-caching-to-use-turborepo": {
      location: "/repo/docs/core-concepts/remote-caching",
    },
    "#do-i-have-to-use-vercel-to-use-turborepo": {
      location: "/repo/docs/core-concepts/remote-caching",
    },
    "#does-turborepo-collect-any-personally-identifiable-information": {
      location: "/repo/docs/telemetry",
    },
    "#does-turborepo--remote-caching-store-my-source-code": {
      location: "/repo/docs/crafting-your-repository/caching",
      hash: "what-gets-cached",
    },
    "#can-i-use-turborepo-with-a-different-remote-cache-provider-other-than-vercel":
      {
        location: "/repo/docs/core-concepts/remote-caching",
      },
    "#how-can-i-retain-fast-refresh-in-my-turborepo-when-using-multiple-nextjs-applications":
      {
        location: "/repo/docs/core-concepts/internal-packages",
        hash: "just-in-time-packages",
      },
    "#what-does-experimental-mean": {
      location: "/governance",
      hash: "#stability-and-release-phases",
    },
  },
  "/repo/docs/getting-started/installation": {
    "#install-globally": { hash: "global-installation" },
    "#install-per-repository": { hash: "repository-installation" },
  },
  "/repo/docs/crafting-your-repository/caching": {
    "#handling-nodejs-versions": {
      location: "/repo/docs/guides/handling-platforms",
    },
    "#hashing": {
      hash: "task-inputs",
    },
    "#handling-platforms-and-other-arbitrary-hash-contributors": {
      location: "/repo/docs/guides/handling-platforms",
      hash: "#operating-systems-architecture-and-other-arbitrary-conditions",
    },
    "#1-write-an-arbitrary-file-to-disk": {
      location: "/repo/docs/guides/handling-platforms",
      hash: "1-write-an-arbitrary-file-to-disk",
    },
    "#2-add-the-file-to-your-gitignore": {
      location: "/repo/docs/guides/handling-platforms",
      hash: "#2-add-the-file-to-your-gitignore",
    },
    "#3-add-the-file-to-the-hash": {
      location: "/repo/docs/guides/handling-platforms",
      hash: "#3-add-the-file-to-the-hash",
    },
    "#4-generate-the-file-before-running-turbo": {
      location: "/repo/docs/guides/handling-platforms",
      hash: "#4-generate-the-file-before-running-turbo",
    },
  },
  "/repo/docs/crafting-your-repository/running-tasks": {
    "#most-tools-dont-optimize-for-speed": {
      location: "/repo/docs/crafting-your-repository/configuring-tasks",
    },
    "#turborepo-can-multitask": {
      location: "/repo/docs/crafting-your-repository/configuring-tasks",
    },
    "#defining-a-pipeline": {
      location: "/repo/docs/crafting-your-repository/configuring-tasks",
    },
    "#running-tasks-from-the-root": {
      location: "/repo/docs/crafting-your-repository/configuring-tasks",
      hash: "registering-root-tasks",
    },
    "#incremental-adoption": {
      location: "/repo/docs/crafting-your-repository/configuring-tasks",
      hash: "when-to-use-root-tasks",
    },
    "#filtering-by-package-name": {
      location: "/repo/docs/crafting-your-repository/running-tasks",
      hash: "filtering-by-package",
    },
  },
  "/repo/docs/crafting-your-repository/using-environment-variables": {
    "#globalenv": { hash: "adding-environment-variables-to-task-hashes" },
    "#pipelinetaskenv": { hash: "adding-environment-variables-to-task-hashes" },
    "#loose--strict-environment-modes": {
      hash: "environment-modes",
    },
    "#wildcards": {
      location: "/repo/docs/reference/configuration",
      hash: "wildcards",
    },
    "#syntax": {
      location: "/repo/docs/reference/configuration",
      hash: "wildcards",
    },
    "#system-environment-variables": {
      location: "/repo/docs/reference/system-environment-variables",
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
  "/repo/docs/crafting-your-repository/configuring-tasks": {
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
  "/repo/docs/reference/configuration": {
    "#glob-specification-for-paths": {
      location: "/repo/docs/reference/globs",
    },
  },
  "/repo/docs/reference": {
    "#global-arguments": {
      hash: "global-flags",
    },
  },
  "/repo/docs/core-concepts/internal-packages": {
    "#anatomy-of-a-package": {
      location: "/repo/docs/crafting-your-repository/structuring-a-repository",
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
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
    redirectList?.[window.location.hash]?.hash;

  const newLocation: string | undefined =
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
    redirectList?.[window.location.hash]?.location;

  if (newHash && newLocation) {
    /* eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- TODO: Fix ESLint Error (#13355)
     * biome-ignore lint/correctness/noVoidTypeReturn: Ignored using `--suppress`
     * */
    return router.push(`${newLocation ?? ""}${newHash ? `#${newHash}` : ""}`);
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
    // eslint-disable-next-line react-hooks/exhaustive-deps -- We only want this hook to run on initial entry to the site.
  }, []);

  return null;
}
