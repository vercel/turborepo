import { DuplicateIcon } from "@heroicons/react/outline";
import copy from "copy-to-clipboard";
import Head from "next/head";
import Image from "next/future/image";
import Link from "next/link";
import toast, { Toaster } from "react-hot-toast";
import edelman from "../../images/edelman.jpeg";
import elad from "../../images/elad.jpeg";
import flavio from "../../images/flavio.jpeg";
import jongold from "../../images/jongold.jpeg";
import ollermi from "../../images/ollermi.jpeg";
import shadcn from "../../images/shadcn.jpeg";
import christian from "../../images/christian.jpeg";
import yangshunz from "../../images/yangshunz.jpeg";
import nmoore from "../../images/nmoore.jpeg";
import joshlarson from "../../images/joshlarson.jpeg";
import paularmstrong from "../../images/paularmstrong.jpeg";

import { Container } from "../Container";
import Tweet, { Mention } from "../Tweet";
import Features from "../Features";
import { Marquee } from "../clients/Marquee";
import { users } from "../clients/users";
import { useTheme } from "next-themes";

export default function Home() {
  const onClick = () => {
    copy("npx create-turbo@latest");
    toast.success("Copied to clipboard");
  };

  const { theme } = useTheme();

  const showcase = users
    .filter((p) => p.pinned)
    .map((user, index) => (
      <Image
        key={`${user.infoLink}-${theme}-${index}-light`}
        src={user.image.replace("/logos", "/logos/color")}
        alt={user.caption}
        width={user.style?.width ?? 100}
        height={75}
        style={{ width: "auto" }}
        priority={true}
        className="inline w-auto mx-8 dark:hidden"
      />
    ));
  const showcaseLight = users
    .filter((p) => p.pinned)
    .map((user, index) => (
      <Image
        key={`${user.infoLink}-${theme}-${index}-dark`}
        src={user.image.replace("/logos", "/logos/white")}
        alt={user.caption}
        width={user.style?.width ?? 100}
        height={75}
        style={{ width: "auto" }}
        priority={true}
        className="hidden w-auto mx-8 dark:inline"
      />
    ));

  return (
    <>
      <Head>
        <title>Turborepo</title>
        <meta
          name="og:description"
          content="Turborepo is a high-performance build system for JavaScript and
          TypeScript codebases"
        />
      </Head>
      <div className="w-auto px-4 pt-16 pb-8 mx-auto sm:pt-24 lg:px-8">
        <h1 className="max-w-5xl text-center mx-auto text-6xl font-extrabold tracking-tighter leading-[1.1] sm:text-7xl lg:text-8xl xl:text-8xl">
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
            <Link href="/docs/getting-started">
              <a className="flex items-center justify-center w-full px-8 py-3 text-base font-medium text-white no-underline bg-black border border-transparent rounded-md dark:bg-white dark:text-black betterhover:dark:hover:bg-gray-300 betterhover:hover:bg-gray-700 md:py-3 md:text-lg md:px-10 md:leading-6">
                Start Building →
              </a>
            </Link>
          </div>
          <div className="relative mt-3 rounded-md sm:mt-0 sm:ml-3">
            <button
              onClick={onClick}
              className="flex items-center justify-center w-full px-8 py-3 font-mono text-sm font-medium text-gray-600 bg-black border border-transparent border-gray-200 rounded-md bg-opacity-5 dark:bg-white dark:text-gray-300 dark:border-gray-700 dark:bg-opacity-5 betterhover:hover:bg-gray-50 betterhover:dark:hover:bg-gray-900 md:py-3 md:text-base md:leading-6 md:px-10"
            >
              npx create-turbo
              <DuplicateIcon className="w-6 h-6 ml-2 -mr-3 text-gray-400" />
            </button>
          </div>
        </div>
      </div>

      <div className="py-16">
        <div className="mx-auto ">
          <p className="pb-8 text-sm font-semibold tracking-wide text-center text-gray-400 uppercase dark:text-gray-500">
            Trusted by teams from around the world
          </p>
          <Marquee>
            {showcase}
            {showcaseLight}
          </Marquee>
        </div>
      </div>

      <div className="relative from-gray-50 to-gray-100">
        <div className="px-4 py-16 mx-auto sm:pt-20 sm:pb-24 lg:max-w-7xl lg:pt-24">
          <h2 className="text-4xl font-extrabold tracking-tight lg:text-5xl xl:text-6xl lg:text-center dark:text-white">
            Build like the best
          </h2>
          <p className="mx-auto mt-4 text-lg font-medium text-gray-400 lg:max-w-3xl lg:text-xl lg:text-center">
            Turborepo reimagines build system techniques used by Facebook and
            Google to remove maintenance burden and overhead.
          </p>
          <Features />
        </div>
      </div>
      <div className="">
        <div className="px-4 py-16 mx-auto sm:pt-20 sm:pb-24 lg:pt-24 lg:px-8">
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
      <div className="sm:py-20 lg:py-24">
        <div className="max-w-4xl px-4 pb-12 mx-auto lg:px-8 ">
          <h2 className="text-4xl font-extrabold leading-tight tracking-tight lg:text-5xl xl:text-6xl md:text-center dark:text-white">
            Loved by badass engineers
          </h2>
        </div>
        <div className="grid gap-4 px-4 mx-auto lg:grid-cols-3 max-w-7xl">
          <div className="space-y-4">
            <Tweet
              url="https://twitter.com/jongold/status/1409714159227326466"
              username="jongold"
              name="Jon Gold"
              date="Jun 28"
              avatar={jongold}
            >
              <Mention>@turborepo</Mention> is the coolest javascript thing
              i&apos;ve seen probably since an early prototype of Next.js
              <br />
              <br />
              javascript fatigue is over
            </Tweet>
            <Tweet
              url="https://twitter.com/flavioukk/status/1405526268615958530"
              username="flavioukk"
              name="Flávio Carvalho"
              date="Jun 17"
              avatar={flavio}
            >
              <Mention>@turborepo</Mention> cache hit in CI is the most
              satisfying thing ever, why hasn&apos;t anyone thought of this
              before lol
            </Tweet>
            <Tweet
              url="https://twitter.com/shadcn/status/1470269932789125123"
              username="yangshunz"
              name="Yangshun Tay"
              date="Dec 12"
              avatar={yangshunz}
            >
              Experimented with <Mention>@turborepo</Mention> on my Flow-based
              4-package monorepo where each package contains lint, test and
              build commands:
              <br />
              <br />
              - lint, test, build all files in series: ~75s
              <br />
              - lerna --parallel: ~62s
              <br />
              - turbo: ~35s (791ms on cache hit)
              <br />
              <br />
              🤯 Impressive results! FULL TURBO!
            </Tweet>
          </div>

          <div className="space-y-4">
            <Tweet
              url="https://twitter.com/paularmstrong/status/1386796930479665158"
              username="paularmstrong"
              name="Paul Armstrong"
              date="Apr 26"
              avatar={paularmstrong}
            >
              Just saw <Mention>@turborepo</Mention> in action and gotta say: it
              looks amazing!
            </Tweet>

            <Tweet
              url="https://twitter.com/edelman215/status/1410388867828654084"
              username="edelman215"
              name="Michael Edelman"
              date="Jun 30"
              avatar={edelman}
            >
              10 runtime-diverse apps, 7 IAC stacks, 6 custom JSII CDK
              constructs, 5 third-party client wrappers, 2 auto-generated
              internal api sdks, a handful of utility/misc packages under
              management, &amp; growing, in 1 monorepo--all in a day&apos;s work
              for <Mention>@turborepo</Mention>
              --no pain, all gain. 😻
            </Tweet>
            <Tweet
              url="https://twitter.com/shadcn/status/1470269932789125123"
              username="shadcn"
              name="shadcn"
              date="Dec 12"
              avatar={shadcn}
            >
              Turborepo is really good at what it does: Ridiculously fast
              builds.
            </Tweet>
            <Tweet
              url="https://twitter.com/n_moore/status/1469344866194788355"
              username="n_moore"
              name="Nate Moore"
              date="Dec 10"
              avatar={nmoore}
            >
              Finally! <Mention>@astrodotbuild</Mention> is now using
              <Mention>@turborepo</Mention>. ♥️⚡️
              <br />
              So glad it&apos;s open source now—congrats to{" "}
              <Mention>@jaredpalmer</Mention> and <Mention>@vercel</Mention> on
              the release!
            </Tweet>
          </div>
          <div className="space-y-4">
            <Tweet
              url="https://twitter.com/ollermi/status/1377458483671543810"
              username="ollermi"
              name="Miguel Oller"
              date="Mar 31"
              avatar={ollermi}
            >
              It&apos;s been a joy to use <Mention>@turborepo</Mention>.{" "}
              <Mention>@jaredpalmer</Mention> is building something truly
              wonderful for the JS community
            </Tweet>
            <Tweet
              url="https://twitter.com/elado/status/1377405777506279425"
              username="elado"
              name="Elad Ossadon"
              date="Mar 31"
              avatar={elad}
            >
              If you build for web, leave everything and go see what{" "}
              <Mention>@jaredpalmer</Mention> is doing with{" "}
              <Mention>@turborepo</Mention>. One of the most exciting pieces of
              tech lately! The hype is real
            </Tweet>

            <Tweet
              url="https://twitter.com/christianjuth/status/1469494057843847169"
              username="christianjuth"
              name="Christian 👨🏼‍💻"
              date="Dec 10"
              avatar={christian}
            >
              Holy wow, I just rewrote my entire Lerna monorepo to use Turborepo
              and SWC, and it took me like maybe 20 minutes. This is insane.
              Literally, everything Vercel has backed/acquired/created makes
              development a little easier. But wow, it just blew my mind how
              easy this all is to use.
            </Tweet>
            <Tweet
              url="https://twitter.com/jplhomer/status/1494080248845062154"
              username="jplhomer"
              name="Josh Larson"
              date="Feb 16"
              avatar={joshlarson}
            >
              <>
                Living that <Mention>@turborepo</Mention> life{" "}
                <span role="img" aria-label="Smiling face with sunglasses">
                  😎
                </span>
              </>
            </Tweet>
          </div>
        </div>
        <Container>
          <div className="px-4 py-16 mx-auto mt-10 sm:max-w-none sm:flex sm:justify-center">
            <div className="space-y-4 sm:space-y-0 sm:mx-auto ">
              <Link href="/docs/getting-started">
                <a className="flex items-center justify-center w-full px-8 py-3 text-base font-medium text-white no-underline bg-black border border-transparent rounded-md dark:bg-white dark:text-black betterhover:dark:hover:bg-gray-300 betterhover:hover:bg-gray-700 md:py-3 md:text-lg md:px-10 md:leading-6">
                  Start Building →
                </a>
              </Link>
            </div>
          </div>
        </Container>
      </div>
      <Toaster position="bottom-right" />
    </>
  );
}
