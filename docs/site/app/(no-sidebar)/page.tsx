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
    <div className="py-12 max-w-6xl px-3 sm:px-6 lg:px-12 mx-auto">
      <Grid
        columns={{
          sm: 1,
          md: 2,
        }}
        className="relative border border-gray-200"
      >
        <div className="absolute -top-[2px]">
          <div className="border-t-[1px] relative w-4 h-4 -left-[.5rem] top-[1px] md:w-8 md:h-8 md:-left-[1rem] md:top-[1px] border-gray-900" />
          <div className="border-l-[1px] relative w-4 h-4 -top-[1.45rem] -left-[1px] md:w-8 md:h-8 md:-top-[2.9rem] md:-left-[1px] border-gray-900" />
        </div>
        <GridCell className="relative border-b col-span-2">
          <DottedLines className="absolute top-0 bottom-0 left-0 right-0 w-full h-full" />
          <div className="relative z-10 flex flex-col justify-center">
            <h1 className="mb-2.5 text-5xl font-medium text-center md:text-heading-64">
              Make ship happen
            </h1>
            <p className="text-gray-900 text-center text-sm md:text-[20px]  text-pretty">
              Turborepo is the build system for JavaScript and TypeScript
              codebases
            </p>
            <div className="flex justify-center my-6 md:mt-4 md:mb-6">
              <div className="relative inline-flex rounded-full">
                <div className="absolute inset-0 rounded-full bg-gradient-to-r from-red-900 to-blue-900"></div>
                <div className="relative rounded-full m-[2px] bg-white dark:bg-black px-6 py-1 md:py-2">
                  <span className="flex flex-col gap-0 md:flex-row md:gap-2 text-sm md:text-[20px] bg-gradient-to-r from-red-900 to-blue-900 bg-clip-text text-transparent">
                    <RemoteCacheCounterClient className="text-center md:text-right" />
                    <span>hours of compute saved</span>
                  </span>
                </div>
              </div>
            </div>
            <div></div>
            <div className="flex flex-col md:flex-row h-fit gap-4 justify-center items-center">
              <Button asChild className="w-full md:w-auto h-[54px]">
                <Link href="/docs">Get started</Link>
              </Button>
              <Snippet
                code="npm i turbo"
                className="flex h-fit w-full md:w-auto items-center border border-[var(--ds-gray-alpha-400)] justify-center font-mono bg-[var(--ds-background-100)]"
              />
            </div>
          </div>
        </GridCell>
        <GridCell className="h-fit col-span-2 border-b">
          <h2 className="mb-1 text-2xl font-medium">Scale your workflows</h2>
          <p className="max-w-prose text-balance text-gray-900 text-copy-16">
            Optimize your local and CI tasks to save years of engineering time
            and compute.
          </p>
          <div className="my-8 grid h-fit gap-y-8 md:grid-cols-3 md:gap-x-8">
            {FEATURES.map((feature) => (
              <div key={feature.title}>
                <div className="flex items-center justify-center">
                  {feature.illustration}
                </div>
                <h3 className="mt-3 font-medium text-[20px] md:mt-6">
                  {feature.title}
                </h3>
                <p className="mt-1.5 text-gray-900 text-copy-16 md:mt-4 text-pretty">
                  {feature.description}
                </p>
              </div>
            ))}
          </div>
        </GridCell>
        <GridCell className="h-fit bg-background-100 col-span-2 border-b">
          <div className="flex flex-col items-start justify-between gap-y-4 md:flex-row">
            <div className="flex flex-col gap-y-1">
              <h2 className="text-2xl font-medium">Simple setup</h2>
              <p className="text-gray-900 text-copy-16 text-pretty">
                Start a new repository or migrate an existing repo incrementally
                in minutes.
              </p>
            </div>
            <Button asChild>
              <Link href="/repo/docs" className="w-full sm:w-auto">
                Read the docs
                <ArrowRight />
              </Link>
            </Button>
          </div>
          <div className="mt-4 grid w-full grid-cols-1 gap-x-4 md:grid-cols-2">
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
        <GridCell className="h-fit col-span-2 border-b">
          <h2 className="text-2xl font-medium text-pretty">
            What builders say about Turborepo
          </h2>

          <Testimonials />
        </GridCell>
        <GridCell className="col-span-2">
          <div className="flex flex-col items-start gap-y-6 md:flex-row md:items-center md:justify-between md:gap-x-6">
            <h2 className="text-2xl font-medium md:text-4xl text-pretty">
              Deploy your Turborepo today.
            </h2>
            <div className="flex flex-col w-full xs:flex-row gap-4 justify-start md:justify-end items-center">
              <Button asChild className="w-full xs:w-auto h-[54px] text-[18px]">
                <Link href="/repo/docs">Get Started</Link>
              </Button>
              <Snippet
                code="npm i turbo"
                className="flex h-fit w-full xs:w-auto items-center border border-[var(--ds-gray-alpha-400)] justify-center font-mono bg-[var(--ds-background-100)]"
              />
            </div>
          </div>
        </GridCell>
      </Grid>
    </div>
  );
}
