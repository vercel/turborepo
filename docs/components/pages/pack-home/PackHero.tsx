import cn from "classnames";
import Image from "next/future/image";
import Link from "next/link";
// import { Marquee } from "../../clients/Marquee";
// import { Clients } from "../../clients/Clients";
import gradients from "./gradients.module.css";
import { HeroText, SectionSubtext } from "./Headings";
import { Gradient } from "./Gradient";
import { useTheme } from "next-themes";
import { FadeIn } from "./FadeIn";
import { CTAButton } from "./CTAButton";

export function PackHero() {
  return (
    <>
      <div className="absolute top-0 dark:from-black from-white to-transparent w-full h-48 z-10 bg-gradient-to-b" />
      <FadeIn className="font-sans w-auto pb-16 pt-[48px] md:pb-24 lg:pb-32 md:pt-16 lg:pt-20 flex justify-between gap-8 items-center flex-col relative z-0">
        <div className="flex justify-center items-center w-full ">
          <Image
            alt="Turbopack"
            src="/images/docs/pack/turbopack-hero-hexagons.svg"
            width={614}
            height={614}
            className="absolute -z-10 min-w-[614px] min-h-[614px]"
          />
          <div className="absolute w-64 h-64 flex items-center justify-center z-50">
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
              alt="Turbopack"
              src={`/images/docs/pack/turbopack-hero-logo-dark.svg`}
              width={120}
              height={120}
              className="hidden dark:block"
            />
            <Image
              alt="Turbopack"
              src={`/images/docs/pack/turbopack-hero-logo-light.svg`}
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
          className="flex max-w-4xl justify-center flex-col items-center gap-5 lg:gap-6 text-center px-6 z-50"
        >
          <h3
            className={cn(
              "font-bold font-wide text-[20px] lg:text-2xl tracking-[0.07em]",
              gradients.turbopackHeaderText
            )}
          >
            TURBOPACK
          </h3>
          <HeroText>The Rust-based successor to Webpack</HeroText>
          <SectionSubtext>
            Turbo is an incremental, distributed bundler optimized for
            JavaScript and TypeScript, written in Rust.
          </SectionSubtext>
        </FadeIn>
        <FadeIn
          delay={0.6}
          className="flex items-center gap-5 max-w-md w-full flex-col px-6 z-50"
        >
          <div className="flex gap-3 w-full ">
            <CTAButton>
              <Link href="/pack/docs">
                <a className="">Get Started</a>
              </Link>
            </CTAButton>
            <CTAButton outline>
              <Link href="/pack/docs">
                <a className="">GitHub</a>
              </Link>
            </CTAButton>
          </div>
          <p className="text-sm text-[#666666]">License: MPL-2.0</p>
        </FadeIn>
        <FadeIn delay={0.8} className="relative w-full">
          <div className="absolute bottom-0 dark:from-black from-white to-transparent w-full h-72 bg-gradient-to-t" />
        </FadeIn>
        {/* Comment this out at the request of Jared */}
        {/* <FadeIn delay={1.6} className="flex w-full justify-center items-center">
          <p
            className={cn(
              "text-xs font-semibold tracking-[0.2em] text-center uppercase mt-8 lg:mt-16 max-w-[300px] lg:max-w-xl px-6",
              gradients.turbopackHeaderText
            )}
          >
            Trusted by teams from around the world
          </p>
        </FadeIn>
        <FadeIn delay={1.6}>
          <Marquee>
            <Clients />
          </Marquee>
        </FadeIn> */}
      </FadeIn>
    </>
  );
}
