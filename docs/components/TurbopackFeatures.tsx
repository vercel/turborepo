import {
  AdjustmentsIcon,
  ArchiveIcon,
  DesktopComputerIcon,
  DownloadIcon,
  ServerIcon,
} from "@heroicons/react/outline";
import { DetailedFeatureLink } from "./Feature";
import { CSSIcon, JSIcon, TSIcon } from "./Icons";

export const TurbopackFeatures = () => {
  return (
    <div className="grid grid-cols-1 mt-12 gap-x-6 gap-y-12 sm:grid-cols-2 lg:mt-16 lg:gap-x-8 lg:gap-y-12">
      <DetailedFeatureLink
        feature={{
          Icon: JSIcon,
          description: `Supports all ESNext features, Browserslist and top-level await.`,
          name: "JavaScript",
        }}
        href="/pack/docs/features/javascript"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: TSIcon,
          description: (
            <>
              Supports TypeScript out of the box, including resolving{" "}
              <code>paths</code> and <code>baseUrl</code>.
            </>
          ),
          name: "TypeScript",
        }}
        href="/pack/docs/features/typescript"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: DownloadIcon,
          description: (
            <>
              Supports <code>require</code>, <code>import</code>, dynamic
              imports and more.
            </>
          ),
          name: "Imports",
        }}
        href="/pack/docs/features/imports"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: DesktopComputerIcon,
          description: `Our optimized dev server supports Hot Module Reloading (HMR) and Fast Refresh.`,
          name: "Dev Server",
        }}
        href="/pack/docs/features/dev-server"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: CSSIcon,
          description: (
            <>
              Supports Global CSS, CSS Modules, postcss-nested and{" "}
              <code>@import</code>.
            </>
          ),
          name: "CSS",
        }}
        href="/pack/docs/features/css"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: ArchiveIcon,
          description:
            "Learn about Next.js, Svelte, Vue and React Server Components support.",
          name: "Frameworks",
        }}
        href="/pack/docs/features/frameworks"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: ServerIcon,
          description: (
            <>
              Supports the <code>/public</code> directory, JSON imports, and
              importing assets via ESM.
            </>
          ),
          name: "Static Assets",
        }}
        href="/pack/docs/features/static-assets"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: AdjustmentsIcon,
          description: (
            <>
              Supports environment variables via <code>.env</code>,{" "}
              <code>.env.local</code>, and more.
            </>
          ),
          name: "Environment Variables",
        }}
        href="/pack/docs/features/environment-variables"
      ></DetailedFeatureLink>
    </div>
  );
};
