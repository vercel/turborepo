import copy from "copy-to-clipboard";
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
  ClipboardCopyIcon,
  DuplicateIcon,
} from "@heroicons/react/outline";
import Head from "next/head";
import Image from "next/image";
import Link from "next/link";
import { Container } from "../Container";
import { Footer } from "../Footer";
import edelman from "../../images/edelman.jpeg";
import elad from "../../images/elad.jpeg";
import flavio from "../../images/flavio.jpeg";
import jongold from "../../images/jongold.jpeg";
import ollermi from "../../images/ollermi.jpeg";
import paularmstrong from "../../images/paularmstrong.jpeg";
import { Window as Terminal } from "../Window";
import { Caret, Prompt } from "../Caret";
import { Keyframes, Frame } from "react-keyframes";
import { Fragment } from "react";
import { useTheme } from "next-themes";
import { useClipboard } from "../useClipboard";
import toast, { Toaster } from "react-hot-toast";
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
    name: "Remote Caching",
    description: `Share a remote build cache with your teammates and CI/CD for even faster builds.`,
    icon: CloudUploadIcon,
  },
  {
    name: "Parallel execution",
    description: `Execute builds using every core at maximum parallelism without wasting idle CPUs.`,
    icon: LightningBoltIcon,
  },
  {
    name: "Zero runtime overhead",
    description: `Turborepo won’t interfere with your runtime code or touch your sourcemaps. `,
    icon: ChipIcon,
  },
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
    name: "Meets you where you’re at",
    description: `Using Lerna? Keep your package publishing workflow and use Turborepo to turbocharge task running.`,
    icon: BeakerIcon,
  },
  {
    name: `Profile in your browser`,
    description: `Generate build profiles and import them in Chrome or Edge to understand which tasks are taking the longest.`,
    icon: ChartBarIcon,
  },
];

const prompt = (
  <Prompt>
    <b>acme</b> [new-logo] ~
  </Prompt>
);
const caret = <Caret />;

const FRAMES = (() => {
  let frames = [];
  let current = [];
  let duration = 0;

  const data = [
    {
      duration: 500,
      0: prompt,
      1: caret,
    },
    {
      duration: 40,
      1: <b>t</b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>tu</b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>tur</b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>turb </b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>turbo</b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>turbo </b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>turbo r</b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>turbo ru</b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>turbo run</b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>turbo run </b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>turbo run b</b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>turbo run bu</b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>turbo run bui</b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>turbo run buil</b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>turbo run build</b>,
      2: caret,
    },
    {
      duration: 40,
      1: <b>turbo run build</b>,
      2: caret,
    },
    {
      duration: 500,
      1: (
        <b>
          turbo run build
          <br />
        </b>
      ),
      2: caret,
    },
    {
      duration: 700,
      1: (
        <b>
          turbo run build
          <br />
        </b>
      ),
      2: "• Remote computation caching enabled(experimental)\n",
      3: "• Running build in 8 packages\n",
      4: "logger:build: cache hit, replaying output 372424b6e1b6199f\n",
      5: "ui:build: cache hit, replaying output 9ba0ecfdffdf2b3b\n",
      6: "ui:build: $ tsup src/index.tsx --format esm,cjs --dts --external react\n",
      7: "ui:build: CLI Building entry: src/index.tsx\n",
      8: "ui:build: CLI Using tsconfig: tsconfig.json\n",
      9: "ui:build: CLI tsup v5.10.1\n",
      10: "ui:build: CLI Target: node12\n",
      11: "ui:build: CJS Build start\n",
      12: "ui:build: ESM Build start\n",
      13: "ui:build: ESM Build success in 93ms\n",
      14: "ui:build: CJS Build success in 107ms\n",
      15: "ui:build: DTS Build start\n",
      16: "ui:build: DTS Build success in 2158ms\n",
      17: "logger:build: $ tsc\n",
      18: "admin:build: cache hit, replaying output 635e129e375ce329\n",
      19: "admin:build: $ vite build\n",
      20: "admin:build: vite v2.6.14 building for production...\n",
      21: "admin:build: transforming...\n",
      22: "admin:build: ✓ 28 modules transformed.\n",
      23: "admin:build: rendering chunks...\n",
      24: "admin:build: dist/assets/logo.ecc203fb.svg    2.61 KiB\n",
      25: "admin:build: dist/index.html                  0.44 KiB\n",
      26: "admin:build: dist/assets/index.fbec93f3.js    2.11 KiB / gzip: 0.91 KiB\n",
      27: "admin:build: dist/assets/index.8b431468.css   0.68 KiB / gzip: 0.44 KiB\n",
      28: "admin:build: dist/assets/vendor.122d9bd3.js   128.46 KiB / gzip: 41.32 KiB\n",
      29: "api:build: cache hit, replaying output 60e0f0b0d8d74393\n",
      30: "api:build: $ tsc\n",
      31: "blog:build: cache hit, replaying output 5db686ecaf7aee13\n",
      32: "blog:build: $ remix build\n",
      33: "blog:build: Building Remix app in production mode...\n",
      34: "blog:build: Built in 628ms\n",
      35: "storefront:build: cache hit, replaying output 0e337510c721a036\n",
      36: "storefront:build: $ next build\n",
      37: "storefront:build: info  - Loaded env from /Users/jared/dev/jaredpalmer/turborepo-starter/packages/storefront/.env\n",
      38: "storefront:build: info  - Using webpack 5. Reason: Enabled by default https://nextjs.org/docs/messages/webpack5\n",
      39: "storefront:build: info  - Checking validity of types...\n",
      40: "storefront:build: info  - Creating an optimized production build...\n",
      41: "storefront:build: next-transpile-modules - global SASS imports only work with a custom _app.js file\n",
      42: "storefront:build: info  - Compiled successfully\n",
      43: "storefront:build: info  - Collecting page data...\n",
      44: "storefront:build: info  - Generating static pages (0/3)\n",
      45: "storefront:build: logger: Hey! This is Home.\n",
      46: "storefront:build: info  - Generating static pages (3/3)\n",
      47: "storefront:build: info  - Finalizing page optimization...\n",
      48: "storefront:build: \n",
      49: "storefront:build: Page                             Size     First Load JS\n",
      50: "storefront:build: ┌ ○ /                            534 B          64.3 kB\n",
      51: "storefront:build: └ ○ /404                         3.17 kB        66.9 kB\n",
      52: "storefront:build: + First Load JS shared by all    63.8 kB\n",
      53: "storefront:build:   ├ chunks/framework.a085b0.js   42 kB\n",
      54: "storefront:build:   ├ chunks/main.5d8b2c.js        20.2 kB\n",
      55: "storefront:build:   ├ chunks/pages/_app.6d0cbf.js  798 B\n",
      56: "storefront:build:   └ chunks/webpack.672781.js     766 B\n",
      57: "storefront:build: \n",
      58: "storefront:build: λ  (Server)  server-side renders at runtime (uses getInitialProps or getServerSideProps)\n",
      59: "storefront:build: ○  (Static)  automatically rendered as static HTML (uses no initial props)\n",
      60: "storefront:build: ●  (SSG)     automatically generated as static HTML + JSON (uses getStaticProps)\n",
      61: "storefront:build:    (ISR)     incremental static regeneration (uses revalidate in getStaticProps)\n",
      62: "storefront:build: \n",
      63: "\n",
      64: " Tasks:    6 successful, 6 total\n",
      65: "Cached:    6 cached, 6 total\n",
      66: "  Time:    194ms >>> FULL TURBO\n",
      67: "\n",
      68: caret,
    },
    {
      duration: 50,
      67: prompt,
      68: caret,
    },
  ];

  for (let i = 0; i < data.length; ++i) {
    for (let line in data[i]) {
      if (line === "duration") {
        duration = data[i][line];
      } else {
        current[line] = data[i][line];
      }
    }

    frames.push(
      <Frame duration={duration} key={`frame-${i}`}>
        {[...current].map((items, idx) => {
          return <Fragment key={idx}>{items}</Fragment>;
        })}
      </Frame>
    );
  }

  return frames;
})();

function Page() {
  const { theme } = useTheme();
  const onClick = () => {
    copy("npx create-turbo");
    toast.success("Copied to clipboard");
  };
  return (
    <>
      <Head>
        <title>Turborepo - Build your monorepo in seconds</title>
      </Head>
      <div className="px-4 py-16 sm:px-6 sm:py-24  lg:px-8  dark:text-white dark:bg-gradient-to-b dark:from-[#000] dark:to-[#111] ">
        <h1 className="text-center text-6xl font-extrabold tracking-tighter leading-[1.1] sm:text-7xl lg:text-8xl xl:text-8xl">
          Monorepos that
          <br className="hidden lg:block" />
          <span className="inline-block text-transparent bg-clip-text bg-gradient-to-r from-red-500 to-blue-500 ">
            make ship happen.
          </span>{" "}
        </h1>
        <p className="max-w-lg mx-auto mt-6 text-xl font-medium leading-tight text-center text-gray-400 sm:max-w-4xl sm:text-2xl md:text-3xl lg:text-4xl">
          Turborepo is a high-performance build system for JavaScript and
          TypeScript codebases.
        </p>
        <div className="max-w-xl mx-auto mt-5 sm:flex sm:justify-center md:mt-8">
          <div className="rounded-md ">
            <Link href="/docs">
              <a
                href="#"
                className="flex items-center justify-center w-full px-8 py-3 text-base font-medium text-white no-underline bg-black border border-transparent rounded-md dark:bg-white dark:text-black betterhover:hover:bg-gray-700 md:py-3 md:text-lg md:px-10 md:leading-6"
              >
                Start building →
              </a>
            </Link>
          </div>
          <div className="relative mt-3 rounded-md sm:mt-0 sm:ml-3">
            <button
              onClick={onClick}
              className="flex items-center justify-center w-full px-8 py-3 font-mono text-sm font-medium text-gray-600 bg-black border border-transparent border-gray-200 rounded-md bg-opacity-5 dark:bg-white dark:text-gray-300 dark:border-gray-700 dark:bg-opacity-5 betterhover:hover:bg-gray-50 md:py-3 md:text-base md:leading-6 md:px-10"
            >
              npx create-turbo
              <DuplicateIcon className="w-6 h-6 ml-2 -mr-3 text-gray-400" />
            </button>
          </div>
        </div>
      </div>
      <div className="relative">
        <div className="absolute inset-0 flex flex-col" aria-hidden="true">
          <div className="flex-1 dark:bg-[#111]" />
          <div className="flex-1 w-full dark:bg-black bg-gray-50" />
        </div>
        <div className="px-4 sm:px-6">
          <div className="relative max-w-lg mx-auto h-[400px]">
            <Terminal
              title="bash"
              className="text-xs text-black dark:text-white"
              height="400"
              white={theme != "dark"}
            >
              <div className="h-[350px] overflow-hidden">
                <Keyframes component="pre" key={`${`running`}-terminal`}>
                  {true
                    ? FRAMES
                    : [
                        <Frame duration={2000} key="static-frame-1">
                          {prompt} {caret}
                        </Frame>,
                        <Frame duration={2000} key="static-frame-2">
                          {prompt} {caret}
                        </Frame>,
                      ]}
                </Keyframes>
              </div>
            </Terminal>
          </div>
        </div>
      </div>

      <div className="py-16 dark:bg-black bg-gray-50">
        <div className="max-w-4xl px-4 mx-auto sm:px-6 lg:px-8">
          <p className="text-sm font-semibold tracking-wide text-center text-gray-400 text-opacity-50 uppercase dark:text-gray-500">
            Trusted by teams from around the world
          </p>

          <div className="grid grid-cols-2 gap-8 mt-6 md:grid-cols-4">
            <div className="flex justify-center col-span-1 filter contrast-50 grayscale dark:opacity-50 md:col-span-2 lg:col-span-1">
              <img
                className="h-6 "
                src="/images/logos/vercel.svg"
                alt="Vercel"
              />
            </div>
            <div className="flex justify-center col-span-1 filter contrast-50 grayscale dark:opacity-50 md:col-span-2 lg:col-span-1">
              <img
                className="h-6 "
                src="/images/logos/lattice.svg"
                alt="Lattice"
              />
            </div>
            <div className="flex justify-center col-span-1 filter contrast-50 grayscale dark:opacity-50 md:col-span-2 lg:col-span-1">
              <img
                className="h-6"
                src="/images/logos/teespring.svg"
                alt="TeeSpring"
              />
            </div>
            <div className="flex justify-center col-span-1 filter contrast-50 grayscale dark:opacity-50 md:col-span-2 lg:col-span-1">
              <img
                className="h-6"
                src="/images/logos/makeswift.svg"
                alt="Makeswift"
              />
            </div>
          </div>
        </div>
      </div>

      <div className="relative dark:bg-black from-gray-50 to-gray-100">
        <div className="max-w-4xl px-4 py-16 mx-auto sm:px-6 sm:pt-20 sm:pb-24 lg:max-w-7xl lg:pt-24 lg:px-8">
          <h2 className="text-4xl font-extrabold tracking-tight lg:text-5xl xl:text-6xl lg:text-center dark:text-white">
            Build like the best
          </h2>
          <p className="mx-auto mt-4 text-lg font-medium text-gray-400 lg:max-w-3xl lg:text-xl lg:text-center">
            Turborepo reimagines build system techniques used by Facebook and
            Google to remove maintenance burden and overhead.
          </p>
          <div className="grid grid-cols-1 mt-12 gap-x-6 gap-y-12 sm:grid-cols-2 lg:mt-16 lg:grid-cols-3 lg:gap-x-8 lg:gap-y-12">
            {features.map((feature) => (
              <div
                className="p-10 bg-white shadow-lg rounded-xl dark:bg-opacity-5 "
                key={feature.name}
              >
                <div>
                  <feature.icon
                    className="h-8 w-8 dark:text-white  rounded-full p-1.5 dark:bg-white dark:bg-opacity-10 bg-black bg-opacity-5 text-black"
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
      <div className="dark:bg-black">
        <div className="px-4 py-16 mx-auto sm:px-6 sm:pt-20 sm:pb-24 lg:pt-24 lg:px-8">
          <h2 className="max-w-4xl mx-auto pb-6 text-5xl font-extrabold  tracking-tight lg:text-6xl xl:text-7xl leading-[1.25!important] md:text-center dark:text-white">
            Scaling your monorepo shouldn&apos;t be so difficult
          </h2>
          <div className="max-w-2xl mx-auto lg:mt-2 dark:text-gray-400">
            <p className="mb-6 text-lg leading-normal text-current lg:text-xl">
              Monorepos are incredible for productivity, especially on the
              frontend, but the tooling can be a nightmare. There&apos;s a lot
              of stuff to do (and things to mess up). Nothing &ldquo;just
              works.&rdquo; It&apos;s become completely normal to waste entire
              days or weeks on plumbing—tweaking configs, writing one-off
              scripts, and stitching stuff together.
            </p>
            <p className="mb-6 text-lg leading-normal text-current lg:text-xl">
              We need something else.
            </p>
            <p className="mb-6 text-lg leading-normal text-current lg:text-xl">
              A fresh take on the whole setup. Designed to glue everything
              together. A toolchain that works for you and not against you. With
              sensible defaults, but even better escape hatches. Built with the
              same techniques used by the big guys, but in a way that
              doesn&apos;t require PhD to learn or a staff to maintain.
            </p>
            <p className="mb-6 text-lg leading-normal text-current lg:text-xl">
              <b className="relative inline-block text-transparent bg-clip-text bg-gradient-to-r from-blue-500 to-red-500">
                With Turborepo, we&apos;re doing just that.
              </b>{" "}
              We&apos;re abstracting the complex configuration needed for most
              monorepos into a single cohesive build system—giving you a world
              class development experience without the maintenance burden.
            </p>
          </div>
          <div className="flex items-center max-w-2xl py-4 mx-auto space-x-4">
            <div className="mt-4">
              <Image
                src="/images/people/jaredpalmer_headshot.jpeg"
                height={90}
                width={90}
                className="block mr-6 rounded-full"
                alt="Jared Palmer"
              />
            </div>
            <div className="flex flex-col h-full space-y-3">
              <div className="-mb-4 dark:hidden">
                <Image
                  src="/images/home/jared_signature_2.png"
                  height={75}
                  width={200}
                  alt="Jared Palmer"
                  className="block w-[200px] "
                />
              </div>
              <div className="hidden -mb-4 dark:block">
                <Image
                  src="/images/home/jared_signature.png"
                  height={75}
                  width={200}
                  className="block w-[200px] "
                  alt="Jared Palmer"
                />
              </div>
              <div className="inline-flex items-center ">
                <a
                  href="https://twitter.com/jaredpalmer"
                  target="_blank"
                  className="font-bold text-gray-400 no-underline"
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
      <div className="bg-gray-50 dark:bg-gradient-to-b dark:bg-black sm:py-20 lg:py-24">
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
                  satisfying thing ever, why hasn&apos;t anyone thought of this
                  before lol
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
                  management, &amp; growing, in 1 monorepo--all in a day&apos;s
                  work for <Mention>@turborepo</Mention>
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
                  <Mention>@turborepo</Mention>. One of the most exciting pieces
                  of tech lately! The hype is real
                </>
              }
            />
          </div>
        </div>
        <Container>
          <div className="max-w-sm py-16 mx-auto mt-10 sm:max-w-none sm:flex sm:justify-center">
            <div className="space-y-4 sm:space-y-0 sm:mx-auto ">
              <Link href="/docs">
                <a className="flex items-center justify-center w-full px-8 py-3 text-base font-medium text-white no-underline bg-black border border-transparent rounded-md dark:bg-white dark:text-black betterhover:hover:bg-gray-700 md:py-3 md:text-lg md:px-10 md:leading-6">
                  Start Building →
                </a>
              </Link>
            </div>
          </div>
        </Container>
      </div>
      <Footer />
      <Toaster position="bottom-right" />
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
    <div className="flex p-4 bg-white rounded-md shadow-xl dark:bg-opacity-10">
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
