import Image from "next/image";
import cn from "classnames";
import Link from "next/link";
import { HeroText } from "../home-shared/Headings";
import gradients from "../home-shared/gradients.module.css";
import { FadeIn } from "../home-shared/FadeIn";
import { CTAButton } from "../home-shared/CTAButton";
import { Gradient } from "../home-shared/Gradient";

export function PackLetter() {
  return (
    <section className="relative flex flex-col items-center px-6 py-16 font-sans md:py-24 lg:py-32 gap-14">
      <FadeIn>
        <HeroText>
          Let&apos;s move
          <br />
          the web forward
        </HeroText>
      </FadeIn>
      <div className="flex flex-col max-w-xl leading-6 md:text-lg lg:text-lg">
        <FadeIn className="opacity-70">
          <p>
            It&apos;s time for a new beginning in compiler infrastructure for
            the entire web ecosystem. Webpack has been downloaded over 3 billion
            times. It&apos;s become an integral part of building for the web.
            But just like Babel and Terser, it&apos;s time to go all-in on
            native. I joined Vercel and assembled a team of world class
            engineers to build the web&apos;s next generation bundler.
          </p>
          <br />
          <p>
            This team has taken lessons from 10 years of Webpack, combined with
            the innovations in incremental computation from Turborepo and
            Google&apos;s Bazel, and invented an architecture ready to withstand
            the next 10 years.
          </p>
          <br />
          <p>
            With that, we&apos;re excited to introduce Turbopack, our
            Rust-powered successor to Webpack. It will harness the power of our
            build system, Turborepo, for massive performance improvements.
            Turbopack is the new foundation of high-performance bare-metal
            tooling and is now open source—we&apos;re excited to share it with
            you.
          </p>
        </FadeIn>
        <FadeIn
          className="relative h-2 md:h-12 lg:h-12"
          noVertical
          viewTriggerOffset
        >
          <span
            className={cn(
              "w-full h-[1px] -bottom-8 md:-bottom-4 lg:-bottom-4 absolute",
              gradients.letterLine
            )}
          />
        </FadeIn>
        <FadeIn
          className="flex items-end justify-center gap-3 md:self-start md:-ml-4 lg:self-start lg:-ml-4 min-w-[300px]"
          noVertical
          viewTriggerOffset
        >
          <div className="w-24 h-24 min-w-[96px] min-h-[96px] rounded-full border dark:border-white/10 border-black/10 flex items-center justify-center ">
            <Image
              alt="Image of Tobias Koopers"
              className="rounded-full"
              height={64}
              src="/images/people/tobiaskoppers.jpg"
              width={64}
            />
          </div>
          <div className="flex flex-col gap-3 pb-2">
            <Image
              alt="Tobias Koppers hand written signature"
              className="block -mb-3 -ml-3 dark:hidden"
              height={91 + 16}
              src="/images/docs/pack/tobias-signature-light.svg"
              // 16 px added and offset to account for the glow
              width={173 + 16}
            />
            <Image
              alt="Tobias Koppers hand written signature"
              className="hidden -mb-3 -ml-3 dark:block"
              height={91 + 16}
              src="/images/docs/pack/tobias-signature-dark.svg"
              // 16 px added and offset to account for the glow
              width={173 + 16}
            />
            <div className="flex gap-2 flex-wrap text-sm leading-none text-[#888888] max-w-[156px] md:max-w-xl lg:max-w-xl">
              <p className="font-bold">Tobias Koppers</p>
              <p>Creator of Webpack</p>
            </div>
          </div>
        </FadeIn>
      </div>
      <FadeIn className="relative flex justify-center w-full mt-16" noVertical>
        <div className="max-w-[180px] w-full">
          <CTAButton>
            <Link className="block py-3 font-sans" href="/pack/docs">
              Start Building
            </Link>
          </CTAButton>
        </div>
        <Gradient
          className="bottom-[-200px] -z-10 opacity-20"
          conic
          height={300}
          width={1200}
        />
      </FadeIn>
    </section>
  );
}
