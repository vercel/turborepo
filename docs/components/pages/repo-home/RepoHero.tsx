import cn from "classnames";
import Image from "next/future/image";
import Link from "next/link";
import gradients from "../home-shared/gradients.module.css";
import { HeroText, SectionSubtext } from "../home-shared/Headings";
import { Gradient } from "../home-shared/Gradient";
import { FadeIn } from "../home-shared/FadeIn";
import { CTAButton } from "../home-shared/CTAButton";
import copy from "copy-to-clipboard";
import toast from "react-hot-toast";

export function RepoHero() {
  const copyCreateTurbo = () => {
    copy("npx create-turbo@latest");
    toast.success("Copied to clipboard!");
  };

  return (
    <>
      <div className="absolute top-0 z-10 w-full h-48 dark:from-black from-white to-transparent bg-gradient-to-b" />
      <FadeIn className="font-sans w-auto pb-16 pt-[48px] md:pb-24 lg:pb-32 md:pt-16 lg:pt-20 flex justify-between gap-8 items-center flex-col relative z-0">
        <div className="flex items-center justify-center w-full ">
          <div className="absolute z-50 min-w-[614px] min-h-[614px]">
            {/* TODO: On dark mode, there should be a "breathing" gradient inside the inner circle */}
            <Image
              alt="Turborepo"
              src="/images/docs/repo/repo-hero-circles-dark.svg"
              width={614}
              height={614}
              className="hidden dark:block"
            />
            <Image
              alt="Turborepo"
              src="/images/docs/repo/repo-hero-circles-light.svg"
              width={614}
              height={614}
              className="block dark:hidden"
            />
          </div>
          <div className="absolute z-50 flex items-center justify-center w-64 h-64">
            <Gradient
              small
              width={120}
              height={120}
              conic
              className="dark:opacity-100 opacity-40"
            />
          </div>

          <div className="w-[120px] h-[120px] z-50">
            <Image
              alt="Turborepo"
              src={`/images/docs/repo/repo-hero-logo-dark.svg`}
              width={120}
              height={120}
              className="hidden dark:block"
            />
            <Image
              alt="Turborepo"
              src={`/images/docs/repo/repo-hero-logo-light.svg`}
              width={120}
              height={120}
              className="block dark:hidden"
            />
          </div>
          <Gradient
            width={1000}
            height={1000}
            className="top-[-500px] dark:opacity-20 opacity-[0.15]"
            conic
          />
        </div>
        <FadeIn
          delay={0.2}
          className="z-50 flex flex-col items-center justify-center gap-5 px-6 text-center lg:gap-6"
        >
          <h3
            className={cn(
              "font-bold font-wide text-[20px] lg:text-2xl tracking-[0.07em]",
              gradients.turbopackHeaderText
            )}
          >
            TURBOREPO
          </h3>
          <HeroText>
            A task runner that
            <br />
            makes ship happen
          </HeroText>
          <SectionSubtext hero>
            Turborepo is a high-performance task runner for JavaScript and
            TypeScript codebases.
          </SectionSubtext>
        </FadeIn>
        <FadeIn
          delay={0.6}
          className="z-50 flex flex-col items-center w-full max-w-md gap-5 px-6 md:max-w-lg"
        >
          <div className="flex flex-col w-full gap-3 md:!flex-row">
            <CTAButton>
              <Link href="/repo/docs">
                <a className="">Get Started</a>
              </Link>
            </CTAButton>
            <CTAButton outline monospace onClick={copyCreateTurbo}>
              <span>npx create-turbo</span>
              <svg
                xmlns="http://www.w3.org/2000/svg"
                fill="none"
                viewBox="0 0 24 24"
                strokeWidth="1.5"
                aria-hidden="true"
                className="w-6 h-6 ml-2 -mr-3 text-gray-400"
              >
                <defs>
                  <linearGradient
                    id="copy-linear-gradient"
                    x1="14.2057"
                    y1="10.5723"
                    x2="10.8375"
                    y2="9.2848"
                    gradientUnits="userSpaceOnUse"
                  >
                    <stop stop-color="#FF1E56"></stop>
                    <stop offset="1" stop-color="#9C51A1"></stop>
                  </linearGradient>
                </defs>
                <path
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  stroke="url(#copy-linear-gradient)"
                  d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                ></path>
              </svg>
            </CTAButton>
          </div>
          <p className="text-sm text-[#666666]">License: MPL-2.0</p>
        </FadeIn>
        <FadeIn delay={0.8} className="relative w-full">
          <div className="absolute bottom-0 w-full dark:from-black from-white to-transparent h-72 bg-gradient-to-t" />
        </FadeIn>
      </FadeIn>
    </>
  );
}
