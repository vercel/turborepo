import Link from "next/link";
import {
  CommandPromptContent,
  CommandPromptCopy,
  CommandPromptList,
  CommandPromptPrefix,
  CommandPromptRoot,
  CommandPromptSurface,
  CommandPromptTrigger,
  CommandPromptTriggerDivider,
  CommandPromptViewport,
} from "@vercel/geistdocs/components/command-prompt";
import { Button } from "@vercel/geistdocs/components/button";
import { Testimonials } from "@/components/testimonials";
import { RemoteCacheCounterClient } from "@/components/remote-cache-counter/client";
import { createMetadata } from "@/lib/create-metadata";
import { HighlightedCode } from "./highlighted-code";
import { ProviderBadges } from "./graphics/provider-badges";
import { RemoteCacheVisual } from "./graphics/remote-cache-visual";
import { MonorepoVisual } from "./graphics/monorepo-visual";

const FEATURES = [
  {
    title: "Works with any provider",
    description:
      "Integrate with any CI provider to keep every workflow fast as your team and codebase grow.",
    illustration: <ProviderBadges />,
  },
  {
    title: "Remote Caching",
    description:
      "Share task outputs across machines and CI so your team never repeats the same work.",
    illustration: <RemoteCacheVisual />,
  },
  {
    title: "Effortless monorepos",
    description:
      "Define dependable workflows once, then run them consistently across local development and CI.",
    illustration: <MonorepoVisual />,
  },
];

const simpleTurboJson = `{
  "tasks": {
    "build": {
      "dependsOn": ["^build"]
    }
  }
}`;

const remoteCachingCommands = `# Login to Remote Cache
turbo login
# Link to Remote Cache
turbo link

# Run tasks
turbo run build`;

export const metadata = createMetadata({
  description:
    "Turborepo is a build system optimized for JavaScript and TypeScript, written in Rust.",
  canonicalPath: "/",
});

export default function HomePage() {
  return (
    <div className="mx-auto w-full max-w-[1448px] px-4 sm:px-6">
      <section className="relative grid grid-cols-1 items-center gap-12 py-16 sm:py-40 lg:grid-cols-12 lg:gap-0">
        <div className="relative z-1 flex flex-col justify-center lg:col-span-7 lg:pr-16">
          <h1 className="lg:max-w-[700px] text-heading-40 sm:text-heading-48 xl:text-heading-64">
            The build system that scales with your codebase
          </h1>
          <p className="mt-6 max-w-[380px] font-normal text-gray-900 text-copy-18">
            Turborepo is the build system for JavaScript and TypeScript
            codebases
          </p>
          <CommandPromptRoot className="items-start mt-4" defaultValue="agents">
            <CommandPromptList>
              <CommandPromptTrigger value="humans">
                For humans
              </CommandPromptTrigger>
              <CommandPromptTriggerDivider />
              <CommandPromptTrigger value="agents">
                For agents
              </CommandPromptTrigger>
            </CommandPromptList>
            <CommandPromptSurface className="h-10 py-0 pr-1.5">
              <CommandPromptPrefix>$</CommandPromptPrefix>
              <CommandPromptViewport>
                <CommandPromptContent copyValue="npm i turbo" value="humans">
                  npm i turbo
                </CommandPromptContent>
                <CommandPromptContent
                  copyValue="npx skills add vercel/turborepo"
                  value="agents"
                >
                  npx skills add vercel/turborepo
                </CommandPromptContent>
              </CommandPromptViewport>
              <CommandPromptCopy />
            </CommandPromptSurface>
          </CommandPromptRoot>
        </div>
        <div className="relative order-first flex min-h-56 items-start border-gray-800 pt-10 lg:order-last lg:col-span-5 lg:min-h-80 lg:pt-0 mx-auto">
          <div className="relative isolate w-fit rounded-t-[12px] bg-background-100 p-6 pb-20">
            <span
              aria-hidden="true"
              className="pointer-events-none absolute top-1/2 right-full h-[75%] w-7 -translate-y-1/2 opacity-85 [mask-image:radial-gradient(49.27%_54.88%_at_100%_50%,#000_0%,transparent_100%)]"
            >
              <span className="block h-full w-full [background:linear-gradient(180deg,#FF1E56_30%,#0196FF_70%)]" />
            </span>
            <span
              aria-hidden="true"
              className="pointer-events-none absolute top-1/2 left-full h-[75%] w-7 -translate-y-1/2 opacity-85 [mask-image:radial-gradient(49.27%_54.88%_at_0%_50%,#000_0%,transparent_100%)]"
            >
              <span className="block h-full w-full [background:linear-gradient(180deg,#FF1E56_30%,#0196FF_70%)]" />
            </span>
            <span
              aria-hidden="true"
              className="pointer-events-none absolute inset-0 -z-10 rounded-t-[12px] bg-background-200 shadow-[0px_0px_0px_1px_var(--ds-gray-alpha-300),0px_1px_1px_rgba(0,0,0,0.02),0px_16px_24px_-4px_rgba(0,0,0,0.04),0px_32px_48px_-8px_rgba(0,0,0,0.06)]"
            />
            <RemoteCacheCounterClient className="min-w-0 bg-gradient-to-r from-[#FF1E56] to-[#0196FF] bg-clip-text pr-2 font-semibold [--font-weight-semibold:450] text-[70px] lg:text-[80px] xl:text-[95px] text-transparent leading-none tracking-[-0.06em]" />
            <p className="mt-1 text-balance text-copy-18 text-gray-1000">
              hours of compute saved due to remote caching
            </p>
            <div
              aria-hidden="true"
              className="pointer-events-none absolute -right-5 -bottom-[68px] -left-5 h-[110px] bg-linear-to-b from-transparent from-0% to-background-200 to-36%"
            />
          </div>
        </div>
      </section>
      <section className="py-10 sm:py-14">
        <h2 className="mb-1 text-heading-32 lg:text-heading-40">
          Scale your workflows
        </h2>
        <p className="max-w-prose text-balance text-muted-foreground text-base">
          Optimize your local and CI tasks to save years of engineering time and
          compute.
        </p>
        <ul className="my-8 grid list-none gap-x-6 gap-y-6 md:grid-cols-2 md:gap-y-10 lg:grid-cols-3 lg:gap-y-12">
          {FEATURES.map((feature) => (
            <li key={feature.title} className="flex w-full flex-col gap-8">
              <div className="aspect-[40/27] overflow-hidden rounded-xs border border-solid border-gray-300">
                {feature.illustration}
              </div>
              <div className="flex flex-col gap-2">
                <h3 className="font-[450] text-copy-16 text-gray-1000">
                  {feature.title}
                </h3>
                <p className="text-balance text-copy-16 text-gray-900">
                  {feature.description}
                </p>
              </div>
            </li>
          ))}
        </ul>
      </section>
      <section className="py-10 sm:py-14">
        <div className="flex flex-col items-start justify-between gap-y-4 md:flex-row">
          <div className="flex flex-col gap-y-1">
            <h2 className="text-heading-32 lg:text-heading-40">Simple setup</h2>
            <p className="text-muted-foreground text-base leading-6 text-pretty">
              Start a new repository or migrate an existing repo incrementally
              in minutes.
            </p>
          </div>
          <Button asChild className="rounded-full" size="lg" variant="outline">
            <Link href="/repo/docs">Read the docs</Link>
          </Button>
        </div>
        <div className="mt-6 grid w-full grid-cols-1 gap-6 md:gap-44 md:grid-cols-2">
          <HighlightedCode
            caption="Declaring a build task"
            lang="json"
            code={simpleTurboJson}
          />
          <HighlightedCode
            caption="Linking to Remote Cache and running tasks"
            lang="bash"
            code={remoteCachingCommands}
          />
        </div>
      </section>
      <section className="py-10 sm:py-14">
        <h2 className="text-heading-32 sm:text-center sm:text-balance lg:text-heading-40">
          What builders say about Turborepo
        </h2>
        <Testimonials />
      </section>
      <section className="py-10 sm:py-14">
        <div className="flex flex-col items-start gap-y-6 md:flex-row md:items-center md:justify-between md:gap-x-6">
          <h2 className="text-heading-32 lg:text-heading-40">
            Deploy your Turborepo today
          </h2>
          <div className="flex flex-col items-start gap-3 sm:flex-row sm:items-center md:justify-end">
            <Button asChild className="rounded-full" size="lg">
              <Link href="/docs">Get started</Link>
            </Button>
            <CommandPromptRoot
              className="w-auto items-start"
              defaultValue="install"
            >
              <CommandPromptSurface className="h-10 pr-1.5">
                <CommandPromptPrefix>$</CommandPromptPrefix>
                <CommandPromptViewport>
                  <CommandPromptContent copyValue="npm i turbo" value="install">
                    npm i turbo
                  </CommandPromptContent>
                </CommandPromptViewport>
                <CommandPromptCopy />
              </CommandPromptSurface>
            </CommandPromptRoot>
          </div>
        </div>
      </section>
    </div>
  );
}
