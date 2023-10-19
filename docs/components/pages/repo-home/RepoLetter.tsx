import { HeroText } from "../home-shared/Headings";
import Image from "next/image";
import cn from "classnames";
import gradients from "../home-shared/gradients.module.css";
import { FadeIn } from "../home-shared/FadeIn";
import { CTAButton } from "../home-shared/CTAButton";
import Link from "next/link";
import { Gradient } from "../home-shared/Gradient";

export function RepoLetter() {
  return (
    <section className="relative flex flex-col items-center px-6 py-16 font-sans md:py-24 lg:py-32 gap-14">
      <FadeIn>
        <HeroText className="lg:text-[65px]">
          Scaling your Codebase
          <br />
          shouldn&apos;t be so difficult
        </HeroText>
      </FadeIn>
      <div className="flex flex-col max-w-xl leading-6 md:text-lg lg:text-lg">
        <FadeIn className="opacity-70">
          <p>
            The bigger your project grows, the slower it gets. Tasks like
            linting, testing, and building begin to take enormous amounts of
            time.
          </p>
          <br />
          <p>
            If you&apos;re serving multiple applications, you might reach for a
            monorepo. They&apos;re incredible for productivity, especially on
            the frontend, but the tooling can be a nightmare. There&apos;s a lot
            of stuff to do (and things to mess up). Nothing &ldquo;just
            works.&rdquo; It&apos;s become completely normal to waste entire
            days or weeks on plumbing—tweaking configs, writing one-off scripts,
            and stitching stuff together.
          </p>
          <br />
          <p>We need something else.</p>
          <br></br>
          <p>
            A fresh take on the whole setup. Designed to glue everything
            together. A toolchain that works for you and not against you. With
            sensible defaults, but even better escape hatches. Built with the
            same techniques used by the big guys, but in a way that doesn&apos;t
            require PhD to learn or a staff to maintain.
          </p>
          <br />
          <p>With Turborepo, we&apos;re doing just that.</p>
          <br />
          <p>
            We&apos;re building a build system that can keep up with your team.
            You&apos;ll see your CI get faster, duplicated work get cut, and
            your NPM scripts get simpler. You&apos;ll get a world-class
            development environment, without the maintenance burden.
          </p>
        </FadeIn>
        <FadeIn noVertical viewTriggerOffset className="relative h-2 md:h-12">
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
          className="flex items-end justify-center gap-3  md:self-start md:-ml-4 lg:self-start lg:-ml-4 min-w-[300px]"
        >
          <div className="w-24 h-24 min-w-[96px] min-h-[96px] rounded-full border dark:border-white/10 border-black/10 flex items-center justify-center ">
            <Image
              alt="Image of Jared Palmer"
              src="/images/people/jaredpalmer.jpeg"
              width={64}
              height={64}
              className="rounded-full grayscale"
            />
          </div>
          <div className="flex flex-col">
            <Image
              alt="Jared Palmer's hand written signature"
              src="/images/docs/repo/jared-signature-light.svg"
              width={190}
              height={90}
              className="block mt-3 mb-4 ml-3 dark:hidden"
            />
            <Image
              alt="Jared Palmer's hand written signature"
              src="/images/docs/repo/jared-signature-dark.svg"
              width={209}
              height={116}
              className="hidden -mt-2 dark:block"
            />
            <div className="flex gap-2 flex-wrap text-sm leading-none text-[#888888] max-w-[156px] md:max-w-xl lg:max-w-xl">
              <p className="font-bold">Jared Palmer</p>
              <p>Founder of Turborepo</p>
            </div>
          </div>
        </FadeIn>
      </div>
      <FadeIn noVertical className="relative flex justify-center w-full mt-16">
        <div className="max-w-[180px] w-full">
          <CTAButton>
            <Link href="/repo/docs" className="block py-3 font-sans">
              Start Building
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
