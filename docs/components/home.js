import {
  ArrowsExpandIcon,
  BeakerIcon,
  ChartBarIcon,
  ChartPieIcon,
  ChipIcon,
  CloudUploadIcon,
  FingerPrintIcon,
  LightningBoltIcon,
  RefreshIcon,
} from "@heroicons/react/outline";
import Image from "next/image";
import { Layout } from "./Layout";
import edelman from "../images/edelman.jpeg";
import elad from "../images/elad.jpeg";
import flavio from "../images/flavio.jpeg";
import jongold from "../images/jongold.jpeg";
import ollermi from "../images/ollermi.jpeg";
import paularmstrong from "../images/paularmstrong.jpeg";
import Link from "next/link";

const features = [
  {
    name: "Incremental builds",
    description: `Building once is painful enough, Turborepo will remember what you've built and skip the stuff that's already been computed.`,
    icon: RefreshIcon,
  },
  {
    name: "Content-aware hashing",
    description: `Turborepo looks at the contents of your files, not timestamps to figure out what needs to be built.`,
    icon: FingerPrintIcon,
  },
  {
    name: "Cloud caching",
    description: `Share a cloud build cache with your teammates and CI/CD for even faster builds.`,
    icon: CloudUploadIcon,
  },
  {
    name: "Parallel execution",
    description: `Execute builds using every core at maximum parallelism without wasting idle CPUs.`,
    icon: LightningBoltIcon,
  },
  {
    name: "Zero runtime overhead",
    description: `Turborepo doesn't interfere with your runtime code or touch your sourcemaps. It does what it does and then gets out of your way.`,
    icon: ChipIcon,
  },
  // {
  //   name: 'Package manager agnostic',
  //   description: `Turborepo works with Yarn v1, Yarn v2, NPM, and PNPM workspaces.`,
  //   icon: LightningBoltIcon,
  // },
  // {
  //   name: 'Focused installs',
  //   description: `Only install the dependencies you actually need. Works perfectly with Docker layer caching.`,
  //   icon: DownloadIcon,
  // },
  {
    name: "Pruned subsets",
    description: `Speed up PaaS deploys by generating a subset of your monorepo with only what's needed to build a specific target.`,
    icon: ChartPieIcon,
  },
  {
    name: "Task pipelines",
    description: `Define the relationships between your tasks and then let Turborepo optimize what to build and when.`,
    icon: ArrowsExpandIcon,
  },
  {
    name: "Convention-based config",
    description: `Reduce complexity through convention. Fan out configuration with just a few lines of JSON.`,
    icon: BeakerIcon,
  },
  {
    name: `Profile in your browser`,
    description: `Generate build profiles and import them in Chrome or Edge to understand which tasks are taking the longest.`,
    icon: ChartBarIcon,
  },
];

function Page() {
  return (
    <>
      <Layout showCta={false} stripe={false}>
        <div className="px-4 py-16 sm:px-6 sm:py-24  lg:px-8  dark:text-white dark:bg-gradient-to-b dark:from-[#08090D] dark:to-[#131820] ">
          <h1 className="text-center text-6xl font-extrabold tracking-tighter leading-[1.1] sm:text-7xl lg:text-8xl xl:text-8xl">
            Monorepos that
            <br className="hidden lg:block" />
            <span className="inline-block text-transparent bg-clip-text bg-gradient-to-r from-red-500 to-blue-500 ">
              make ship happen.
            </span>{" "}
          </h1>
          <p className="max-w-lg mx-auto mt-6 text-xl font-medium leading-tight text-center text-gray-400 sm:max-w-4xl sm:text-2xl md:text-3xl lg:text-4xl">
            Turborepo is a high-performance build system for modern codebases.
          </p>
          <div className="max-w-sm mx-auto mt-10 sm:max-w-none sm:flex sm:justify-center">
            <div className="space-y-4 sm:space-y-0 sm:mx-auto ">
              <Link href="/docs/getting-started">
                <a className="flex items-center justify-center px-4 py-3 text-lg font-medium text-white no-underline rounded-md bg-gradient-to-r from-red-500 to-blue-500 dark:text-gray-900 hover:bg-gray-50 sm:px-8">
                  Start building →
                </a>
              </Link>
            </div>
          </div>
        </div>
        <div className="relative">
          <div className="absolute inset-0 flex flex-col" aria-hidden="true">
            <div className="flex-1 dark:bg-[#131820]" />
            <div className="flex-1 w-full dark:bg-[#050b13] bg-gray-50" />
          </div>
          <div className="px-4 sm:px-6">
            <div className="relative max-w-screen-xl mx-auto text-center">
              <Image
                width={1152}
                src="/thumbnail.png"
                height={661}
                className="block w-full mx-auto shadow-2xl "
              />
            </div>
          </div>
        </div>

        {/* <div className="dark:bg-[#050b13] bg-gray-50 py-16">
          <div className="px-4 mx-auto max-w-7xl sm:px-6 lg:px-8">
            <p className="text-sm font-semibold tracking-wide text-center text-gray-400 text-opacity-50 uppercase dark:text-gray-500">
              Trusted in production
            </p>

            <div className="grid grid-cols-2 gap-8 mt-6 md:grid-cols-6">
              <div className="flex justify-center col-span-1 filter contrast-50 grayscale dark:opacity-50 md:col-span-2 lg:col-span-1">
                <img
                  className="h-12 text-gray-500"
                  src="/logos/aws.svg"
                  alt="Amazon Web Services"
                />
              </div>
              <div className="flex justify-center col-span-1 filter contrast-50 grayscale dark:opacity-50 md:col-span-2 lg:col-span-1">
                <img
                  className="h-10 text-gray-500"
                  src="/logos/lattice.svg"
                  alt="Lattice"
                />
              </div>
              <div className="flex justify-center col-span-1 filter contrast-50 grayscale dark:opacity-50 md:col-span-2 lg:col-span-1">
                <img className="h-10" src="/logos/marvel.svg" alt="Marvel" />
              </div>
              <div className="flex justify-center col-span-1 filter contrast-50 grayscale dark:opacity-50 md:col-span-2 lg:col-span-1">
                <img
                  className="h-10"
                  src="/logos/makeswift.svg"
                  alt="Makeswift"
                />
              </div>
              <div className="flex justify-center col-span-1 filter contrast-50 grayscale dark:opacity-50 md:col-span-2 lg:col-span-1">
                <img className="h-10" src="/logos/ondeck.svg" alt="On Deck" />
              </div>

              <div className="flex justify-center col-span-1 filter contrast-50 grayscale dark:opacity-50 md:col-span-2 lg:col-span-1">
                <img
                  className="h-12"
                  src="/logos/youhodler.svg"
                  alt="YouHodler"
                />
              </div>
            </div>
          </div>
        </div> */}

        <div className="relative bg-gradient-to-b dark:from-[#050b13] dark:to-[#131820] from-gray-50 to-gray-100">
          <div className="max-w-4xl px-4 py-16 mx-auto sm:px-6 sm:pt-20 sm:pb-24 lg:max-w-7xl lg:pt-24 lg:px-8">
            <h2 className="text-4xl font-extrabold tracking-tight lg:text-5xl xl:text-6xl lg:text-center dark:text-white">
              Why Turborepo?
            </h2>
            <p className="mx-auto mt-4 text-lg font-medium text-gray-400 lg:max-w-3xl lg:text-xl lg:text-center">
              Turborepo has the tools you need to scale your codebase.
            </p>
            <div className="grid grid-cols-1 mt-12 gap-x-6 gap-y-12 sm:grid-cols-2 lg:mt-16 lg:grid-cols-3 lg:gap-x-8 lg:gap-y-12">
              {features.map((feature) => (
                <div
                  className="p-10 bg-white shadow-lg rounded-xl dark:bg-opacity-5 "
                  key={feature.name}
                >
                  <div>
                    <feature.icon
                      className="h-8 w-8 text-white dark:text-gray-900 rounded-full p-1.5 bg-gradient-to-br from-blue-500 to-red-500 "
                      aria-hidden="true"
                    />
                  </div>
                  <div className="mt-4">
                    <h3 className="text-lg font-medium dark:text-white">
                      {feature.name}
                    </h3>
                    <p className="mt-2 text-base font-medium text-gray-500 dark:text-gray-400">
                      {feature.description}
                    </p>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>
        <div className="dark:bg-[#050b13]">
          <div className="max-w-4xl px-4 py-16 mx-auto sm:px-6 sm:pt-20 sm:pb-24 lg:pt-24 lg:px-8">
            <h2 className="mb-6 text-4xl font-extrabold leading-tight tracking-tight lg:text-5xl xl:text-6xl dark:text-white">
              Scaling your monorepo shouldn&apos;t be so difficult
            </h2>
            <div className="max-w-4xl mx-auto lg:mt-2 dark:text-gray-200">
              <p className="mb-6 text-xl text-current">
                Monorepos are incredible for productivity, especially on the
                frontend, but the tooling can be a nightmare. There&apos;s a lot
                of stuff to do (and things to mess up). Nothing &ldquo;just
                works.&rdquo; It&apos;s become completely normal to waste entire
                days or weeks on plumbing—tweaking configs, writing one-off
                scripts, and stitching stuff together.
              </p>
              <p className="mb-6 text-xl text-current">
                We need something else.
              </p>
              <p className="mb-6 text-xl text-current">
                A fresh take on the whole setup. Designed to glue everything
                together. A toolchain that works for you and not against you.
                With sensible defaults, but even better escape hatches. Built
                with the same techniques used by the big guys, but in a way that
                doesn&apos;t require PhD to learn or a staff to maintain.
              </p>
              <p className="mb-6 text-xl text-current">
                <b>With Turborepo, we&apos;re doing just that.</b> We&apos;re
                abstracting the complex configuration needed for most monorepos
                into a single cohesive build system—giving you a world class
                development experience without the maintenance burden.
              </p>
            </div>
            <div className="flex items-center py-4 space-x-4">
              <div className="mt-4">
                <Image
                  src="/jaredpalmer_headshot.jpeg"
                  height={90}
                  width={90}
                  className="block mr-6 rounded-full"
                  alt="Jared Palmer"
                />
              </div>
              <div className="flex flex-col h-full space-y-2">
                <div className="-mb-4">
                  <Image
                    src="/jared_signature.png"
                    height={75}
                    width={200}
                    className="block w-[200px]"
                    alt="Jared Palmer"
                  />
                </div>
                <div className="inline-flex items-center ">
                  <a
                    href="https://twitter.com/jaredpalmer"
                    target="_blank"
                    className="font-bold text-gray-400"
                    rel="noopener noreferrer"
                  >
                    Jared Palmer
                  </a>
                  <div className="ml-2 text-gray-500">Founder of Turborepo</div>
                </div>
              </div>
            </div>
          </div>
        </div>
        <div className="bg-gray-50 dark:bg-gradient-to-b dark:from-[#050b13] dark:to-[#131820] sm:py-20 lg:py-24">
          <div className="max-w-4xl px-4 pb-12 mx-auto sm:px-6 lg:px-8 ">
            <h2 className="text-4xl font-extrabold leading-tight tracking-tight lg:text-5xl xl:text-6xl md:text-center dark:text-white">
              Loved by badass engineers
            </h2>
          </div>
          <div className="grid gap-4 px-4 mx-auto sm:px-6 lg:grid-cols-3 max-w-7xl">
            <div className="space-y-4">
              <Tweet
                url="https://twitter.com/jongold/status/1409714159227326466"
                username="jongold"
                name="Jon Gold"
                date="Jun 28"
                avatar={jongold}
                text={
                  <>
                    . <Mention>@turborepo</Mention> is the coolest javascript
                    thing i&apos;ve seen probably since an early prototype of
                    Next.js
                    <br />
                    <br />
                    javascript fatigue is over
                  </>
                }
              />
              <Tweet
                url="https://twitter.com/flavioukk/status/1405526268615958530"
                username="flavioukk"
                name="Flávio Carvalho"
                date="Jun 17"
                avatar={flavio}
                text={
                  <>
                    . <Mention>@turborepo</Mention> cache hit in CI is the most
                    satisfying thing ever, why hasn&apos;t anyone thought of
                    this before lol
                  </>
                }
              />
            </div>

            <div className="space-y-4">
              <Tweet
                url="https://twitter.com/paularmstrong/status/1386796930479665158"
                username="paularmstrong"
                name="Paul Armstrong"
                date="Apr 26"
                avatar={paularmstrong}
                text={
                  <>
                    Just saw <Mention>@turborepo</Mention> in action and gotta
                    say: it looks amazing!
                  </>
                }
              />

              <Tweet
                url="https://twitter.com/edelman215/status/1410388867828654084"
                username="edelman215"
                name="Michael Edelman"
                date="Jun 30"
                avatar={edelman}
                text={
                  <>
                    10 runtime-diverse apps, 7 IAC stacks, 6 custom JSII CDK
                    constructs, 5 third-party client wrappers, 2 auto-generated
                    internal api sdks, a handful of utility/misc packages under
                    management, &amp; growing, in 1 monorepo--all in a
                    day&apos;s work for <Mention>@turborepo</Mention>
                    --no pain, all gain. 😻
                  </>
                }
              />
            </div>
            <div className="space-y-4">
              <Tweet
                url="https://twitter.com/ollermi/status/1377458483671543810"
                username="ollermi"
                name="Miguel Oller"
                date="Mar 31"
                avatar={ollermi}
                text={
                  <>
                    It&apos;s been a joy to use <Mention>@turborepo</Mention>.{" "}
                    <Mention>@jaredpalmer</Mention> is building something truly
                    wonderful for the JS community
                  </>
                }
              />
              <Tweet
                url="https://twitter.com/elado/status/1377405777506279425"
                username="elado"
                name="Elad Ossadon"
                date="Mar 31"
                avatar={elad}
                text={
                  <>
                    If you build for web, leave everything and go see what{" "}
                    <Mention>@jaredpalmer</Mention> is doing with{" "}
                    <Mention>@turborepo</Mention>. One of the most exciting
                    pieces of tech lately! The hype is real
                  </>
                }
              />
            </div>
          </div>
        </div>
      </Layout>
    </>
  );
}

function Mention({ children }) {
  return (
    <a
      href={`https://twitter.com/${children.replace("@", "")}`}
      target="_blank"
      rel="noopener noreferrer"
      className="inline-block text-[#35ACDF]"
    >
      {children}
    </a>
  );
}

function Tweet({ url, username, name, text, avatar, date }) {
  return (
    <div className="flex p-4 bg-white rounded-md shadow-xl bg-opacity-10">
      <div className="flex-shrink-0 mr-4">
        <Image
          className="w-12 h-12 rounded-full"
          width={42}
          height={42}
          src={avatar}
          alt={`${name} twitter avatar`}
        />
      </div>
      <div>
        <div className="flex items-center space-x-1 text-sm">
          <h4 className="font-medium dark:text-white">{name}</h4>
          <div className="truncate dark:text-gray-400">@{username}</div>
          <div className="dark:text-gray-500 md:hidden xl:block">• {date}</div>
        </div>
        <div className="mt-1 text-sm dark:text-gray-200">{text}</div>
      </div>
    </div>
  );
}

export default Page;
