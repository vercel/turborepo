import { HeroText } from "./Headings";
import Image from "next/future/image";
import cn from "classnames";
import gradients from "./gradients.module.css";
import { FadeIn } from "./FadeIn";
import { CTAButton } from "./CTAButton";
import Link from "next/link";
import { Gradient } from "./Gradient";

export function PackLetter() {
  return (
    <section className="font-sans relative px-6 py-16 md:py-24 lg:py-32 items-center flex flex-col gap-14">
      <FadeIn>
        <HeroText>
          Let&apos;s move
          <br />
          the web forward
        </HeroText>
      </FadeIn>
      <div className="flex flex-col max-w-xl md:text-lg lg:text-lg leading-6">
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
            With that, we&apos;re excited to introduce Turbopack, our Rust-based
            successor to Webpack. It will harness the power of our build system,
            Turborepo, for massive performance improvements. Turbopack is the
            new foundation of high-performance bare-metal tooling and is now
            open source—we&apos;re excited to share it with you.
          </p>
        </FadeIn>
        <FadeIn
          noVertical
          viewTriggerOffset
          className="relative h-2 md:h-12 lg:h-12"
        >
          <span
            className={cn(
              "w-full h-[1px] -bottom-8 md:-bottom-4 lg:-bottom-4 absolute",
              gradients.letterLine
            )}
          />
        </FadeIn>
        <FadeIn
          viewTriggerOffset
          noVertical
          className="flex items-end justify-center gap-3 md:self-start md:-ml-4 lg:self-start lg:-ml-4 min-w-[300px]"
        >
          <div className="w-24 h-24 min-w-[96px] min-h-[96px] rounded-full border dark:border-white/10 border-black/10 flex items-center justify-center ">
            <Image
              alt="Image of Tobias Koopers"
              src="/images/people/tobias.jpg"
              width={64}
              height={64}
              className="rounded-full"
            />
          </div>
          <div className="flex flex-col pb-2 gap-3">
            <Image
              alt="Tobias Koppers hand written signature"
              src="/images/docs/pack/tobias-signature-light.svg"
              // 16 px added and offset to account for the glow
              width={173 + 16}
              height={91 + 16}
              className="-ml-3 -mb-3 block dark:hidden"
            />
            <Image
              alt="Tobias Koppers hand written signature"
              src="/images/docs/pack/tobias-signature-dark.svg"
              // 16 px added and offset to account for the glow
              width={173 + 16}
              height={91 + 16}
              className="-ml-3 -mb-3 hidden dark:block"
            />
            <div className="flex gap-2 flex-wrap text-sm leading-none text-[#888888] max-w-[156px] md:max-w-xl lg:max-w-xl">
              <p className="font-bold">Tobias Koppers</p>
              <p>Creator of Webpack</p>
            </div>
          </div>
        </FadeIn>
      </div>
      <FadeIn noVertical className="w-full flex justify-center  mt-16 relative">
        <div className="max-w-[180px] w-full">
          <CTAButton>
            <Link href="/pack/docs">
              <a className="font-sans">Start Building</a>
            </Link>
          </CTAButton>
        </div>
        <Gradient
          width={1200}
          height={300}
          className="bottom-[-200px] -z-10 opacity-20"
          conic
        />
      </FadeIn>
    </section>
  );
}
