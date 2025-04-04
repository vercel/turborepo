import { Button } from "#/components/button";
import { Grid } from "@/components/grid/grid";
import { GridCell } from "@/components/grid/grid-cell";
import { Snippet } from "@/components/snippet";
import { DynamicCodeBlock } from "fumadocs-ui/components/dynamic-codeblock";
import { Testimonials } from "./testimonials";
import { ArrowRight } from "#/components/icons/arrow-right";
import type { Metadata } from "next";
import { createCssVariablesTheme } from "shiki";

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
    illustration: (
      <div className="w-[200px] h-[200px]">
        <p>Image #1</p>
      </div>
    ),
  },
  {
    title: "Remote Caching",
    description: "Never do the same work twice",
    illustration: (
      <div className="w-[200px] h-[200px]">
        <p>Image #2</p>
      </div>
    ),
  },
  {
    title: "Effortless monorepos",
    description: "Easily define your workflows for local development and CI",
    illustration: (
      <div className="w-[200px] h-[200px]">
        <p>Image #3</p>
      </div>
    ),
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

export default async function HomePage() {
  return (
    <div className="py-12 max-w-6xl mx-auto">
      <Grid
        columns={{
          sm: 1,
          md: 2,
        }}
        className="border border-gray-200"
      >
        <GridCell className="border-b border-r">
          <div className="flex flex-col justify-center">
            <h1 className="mb-2.5 text-5xl font-semibold md:text-heading-64">
              Make ship happen
            </h1>
            <p className="mb-6 font-medium text-gray-900 text-label-20 md:mb-12">
              The build system for JavaScript and TypeScript codebases
            </p>
            <div className="flex h-fit gap-x-4 items-center">
              <Button asChild className="h-[48px] text-[18px]">
                <a href="/docs">Get started</a>
              </Button>
              <Snippet
                code="npm i turbo"
                className="flex h-fit items-center border border-[var(--ds-gray-alpha-400)] justify-center font-mono bg-[var(--ds-background-100)]"
              />
            </div>
          </div>
        </GridCell>
        <GridCell className="relative sm:!mb-0 border-b">
          <p>A hero image goes here!</p>
        </GridCell>
        <GridCell className="h-fit col-span-2 border-b">
          <h2 className="mb-1 text-2xl font-semibold">Scale your workflows</h2>
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
                <h3 className="mt-3 text-heading-24 md:mt-6">
                  {feature.title}
                </h3>
                <p className="mt-1.5 text-gray-900 text-copy-16 md:mt-4">
                  {feature.description}
                </p>
              </div>
            ))}
          </div>
        </GridCell>
        <GridCell className="h-fit bg-background-100 col-span-2 border-b">
          <div className="flex flex-col items-start justify-between gap-y-4 md:flex-row">
            <div className="flex flex-col gap-y-1">
              <h2 className="text-2xl font-semibold">Simple setup</h2>
              <p className="text-gray-900 text-copy-16">
                Start a new repository or migrate an existing repo incrementally
                in minutes
              </p>
            </div>
            <Button asChild>
              <a href="/repo/docs">
                Read the Docs
                <ArrowRight />
              </a>
            </Button>
          </div>
          <div className="mt-4 grid w-full grid-cols-1 gap-x-4 md:grid-cols-2">
            <div className="mb-6 md:mb-0">
              <DynamicCodeBlock
                lang="json"
                code={simpleTurboJson}
                options={{
                  themes: {
                    light: theme,
                    dark: theme,
                  },
                }}
              />
              <span className="text-xs text-gray-900">
                Declaring a build task
              </span>
            </div>
            <div>
              <DynamicCodeBlock
                lang="bash"
                code={remoteCachingCommands}
                options={{
                  themes: {
                    light: theme,
                    dark: theme,
                  },
                }}
              />
              <span className="text-xs text-gray-900">
                Linking to Remote Cache and running tasks
              </span>
            </div>
          </div>
        </GridCell>
        <GridCell className="h-fit col-span-2 border-b">
          <h2 className="text-2xl font-semibold">
            What builders say about Turborepo
          </h2>

          <Testimonials />
        </GridCell>
        <GridCell className="col-span-2">
          <div className="flex flex-col items-start gap-y-6 md:flex-row md:items-center md:justify-between md:gap-x-6">
            <h2 className="text-2xl font-semibold md:text-4xl">
              Deploy your Turborepo today.
            </h2>
            <div className="flex gap-x-4 items-center">
              <Button asChild className="h-[48px] text-[18px]">
                <a href="/repo/docs">Get Started</a>
              </Button>
              <Snippet
                code="npm i turbo"
                className="flex h-fit items-center border border-[var(--ds-gray-alpha-400)] justify-center font-mono bg-[var(--ds-background-100)]"
              />
            </div>
          </div>
        </GridCell>
      </Grid>
    </div>
  );
}
