import { DynamicCodeBlock } from "fumadocs-ui/components/dynamic-codeblock";
import type { Metadata } from "next";
import { createCssVariablesTheme } from "shiki";
import Link from "next/link";
import { Button } from "#components/button.tsx";
import { Grid } from "#components/grid/grid.tsx";
import { GridCell } from "#components/grid/grid-cell.tsx";
import { Snippet } from "#components/snippet.tsx";
import { Testimonials } from "#components/testimonials.tsx";
import { ArrowRight } from "#components/icons/arrow-right.tsx";
import { RemoteCacheCounterClient } from "#components/remote-cache-counter/client.tsx";
import { CiProviders } from "./graphics/providers";
import { RemoteCachingGraphic } from "./graphics/remote-caching";
import { EffortlessGraphic } from "./graphics/effortless";
import { DottedLines } from "./graphics/dotted-lines";

// Copied from source.config.ts
const theme = createCssVariablesTheme({
  name: "css-variables",
  variablePrefix: "--shiki-",
  variableDefaults: {},
});

const FEATURES = [
  {
    title: "Works with any provider",
    description: "Integrate with any CI provider for speed at all scales",
    illustration: <CiProviders />,
  },
  {
    title: "Remote Caching",
    description: "Never do the same work twice",
    illustration: <RemoteCachingGraphic />,
  },
  {
    title: "Effortless monorepos",
    description: "Easily define your workflows for local development and CI",
    illustration: <EffortlessGraphic />,
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

export const metadata: Metadata = {
  alternates: { canonical: "https://turbo.build" },
};

export default function HomePage() {
  return (
    <div className="py-6 max-w-6xl px-3 sm:px-6 md:py-12 lg:px-12 mx-auto">
      <Grid
        columns={{
          sm: 1,
          md: 2,
        }}
        className="relative border border-gray-400"
      >
        <div className="absolute -top-[2px]">
          <div className="border-t-[1px] absolute w-[11px] h-[11px] -left-[6px] top-[1px] md:w-[21px] md:h-[21px] md:-left-[11px] md:top-[1px] border-gray-600" />
          <div className="border-l-[1px] absolute w-[11px] h-[11px] -top-[4px] -left-[1px] md:w-[21px] md:h-[21px] md:-top-[11px] md:-left-[1px] border-gray-600" />
        </div>
        <GridCell className="relative border-b col-span-2 px-6 py-12 xs:px-6 xs:py-12 md:p-16">
          <DottedLines className="absolute top-0 bottom-0 left-0 right-0 overflow-hidden text-center flex items-center justify-center" />
          <div className="relative z-1 flex flex-col justify-center">
            <h1 className="mb-4 text-6xl font-semibold tracking-tighter text-center md:text-7xl">
              Make ship happen
            </h1>
            <p className="max-w-[380px] m-auto mb-4 font-normal text-center text-gray-900 text-lg md:text-xl">
              Turborepo is the build system for JavaScript and TypeScript
              codebases
            </p>
            <div className="flex justify-center mt-2 mb-10">
              <div className="relative inline-flex w-full xs:w-auto">
                <div className="absolute inset-0 rounded-lg xs:rounded-[22px] bg-gradient-to-r from-[#FF1E56] to-[#0196FF] w-full xs:w-auto"></div>
                <div className="relative text-center rounded-md xs:rounded-[20px] m-[2px] bg-background-100 dark:bg-black px-4 py-1.5 md:px-5 md:py-0.5 w-full xs:w-auto">
                  <span className="flex flex-col gap-0 items-center xs:flex-row sm:gap-1 text-base sm:text-xl leading-tight bg-gradient-to-r from-[#FF1E56] to-[#0196FF] bg-clip-text text-transparent">
                    <RemoteCacheCounterClient className="" />
                    <span>hours of compute saved</span>
                  </span>
                </div>
              </div>
            </div>
            <div className="w-full flex flex-wrap h-fit gap-3 2xs:gap-2 sm:gap-4 justify-center items-center">
              <Button asChild className="text-sm sm:h-12 sm:text-base">
                <Link href="/docs">Get started</Link>
              </Button>
              <Snippet
                code="npm i turbo"
                className="flex h-fit min-w-[160px] max-w-[180px] xs:w-auto sm:h-12 items-center border border-[var(--ds-gray-alpha-400)] justify-start font-mono bg-[var(--ds-background-100)]"
              />
            </div>
          </div>
        </GridCell>
        <GridCell className="border-0 h-fit col-span-2 px-6 py-14 xs:px-6 xs:py-10 md:px-9 lg:px-12">
          <h2 className="mb-1 text-[32px] font-semibold tracking-tighter">
            Scale your workflows
          </h2>
          <p className="max-w-prose text-balance text-gray-900 text-base">
            Optimize your local and CI tasks to save years of engineering time
            and compute.
          </p>
          <div className="my-8 grid h-fit gap-y-12 md:grid-cols-3 md:gap-x-8">
            {FEATURES.map((feature) => (
              <div key={feature.title} className="w-full">
                {feature.illustration}
                <h3 className="mt-2 text-2xl font-semibold tracking-tighter">
                  {feature.title}
                </h3>
                <p className="mt-1.5 text-gray-900 text-base md:mt-2 text-pretty">
                  {feature.description}
                </p>
              </div>
            ))}
          </div>
        </GridCell>
        <GridCell className="col-span-2 px-6 py-14 xs:px-6 xs:py-10 md:px-9 lg:px-12">
          <div className="flex flex-col items-start justify-between gap-y-4 md:flex-row">
            <div className="flex flex-col gap-y-1">
              <h2 className="text-[32px] font-semibold tracking-tighter">
                Simple setup
              </h2>
              <p className="text-gray-900 text-copy-16 text-pretty">
                Start a new repository or migrate an existing repo incrementally
                in minutes.
              </p>
            </div>
            <Button
              asChild
              className="text-sm sm:h-12 sm:text-base"
              variant="outline"
            >
              <Link href="/repo/docs">
                Read the docs
                <ArrowRight />
              </Link>
            </Button>
          </div>
          <div className="mt-6 grid w-full grid-cols-1 gap-x-4 md:grid-cols-2">
            <div className="mb-6 md:mb-0">
              <DynamicCodeBlock
                lang="json"
                code={simpleTurboJson}
                options={
                  // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment -- Types are fixed in a higher version of Fumadocs than we are on
                  {
                    themes: {
                      light: theme,
                      dark: theme,
                    },
                    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- Types are fixed in a higher version of Fumadocs than we are on
                  } as any
                }
              />
              <span className="text-xs text-gray-900">
                Declaring a build task
              </span>
            </div>
            <div>
              <DynamicCodeBlock
                lang="bash"
                code={remoteCachingCommands}
                options={
                  // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment -- Types are fixed in a higher version of Fumadocs than we are on
                  {
                    themes: {
                      light: theme,
                      dark: theme,
                    },
                    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- Types are fixed in a higher version of Fumadocs than we are on
                  } as any
                }
              />
              <span className="text-xs text-gray-900">
                Linking to Remote Cache and running tasks
              </span>
            </div>
          </div>
        </GridCell>
        <GridCell className="col-span-2 px-6 py-14 xs:px-6 xs:py-10 md:px-9 lg:px-12 border-b">
          <h2 className="text-[32px] font-semibold tracking-tighter">
            What builders say about Turborepo
          </h2>

          <Testimonials />
        </GridCell>
        <GridCell className="col-span-2 px-6 py-14 xs:px-6 xs:py-10 md:px-9 lg:px-12">
          <div className="flex flex-col items-start gap-y-6 md:flex-row md:items-center md:justify-between md:gap-x-6">
            <h2 className="text-[32px] font-semibold tracking-tighter md:text-[40px]">
              Deploy your Turborepo today.
            </h2>
            <div className="flex flex-wrap gap-3 justify-start md:justify-end items-center">
              <Button asChild className="text-sm sm:h-12 sm:text-base">
                <Link href="/docs">Get started</Link>
              </Button>
              <Snippet
                code="npm i turbo"
                className="flex h-fit min-w-[160px] max-w-[180px] xs:w-auto sm:h-12 items-center border border-[var(--ds-gray-alpha-400)] justify-start font-mono bg-[var(--ds-background-100)]"
              />
            </div>
          </div>
        </GridCell>
      </Grid>
    </div>
  );
}
