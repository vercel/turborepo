"use client";

import React, { useState } from "react";
import { cn } from "@/components/cn";
import Link from "next/link";
import { motion } from "framer-motion";
import { Clients } from "@/app/_clients/clients";
import { FadeIn } from "@/app/_components/home-shared/fade-in";
import { PackLogo } from "@/app/_components/logos/pack-logo";
import { RepoLogo } from "@/app/_components/logos/repo-logo";
import { TurboheroBackground } from "@/app/_components/turbohero-background";
import { Turborepo } from "@/app/_components/turborepo";
import { Turbopack } from "@/app/_components/turbopack";
import { PRODUCT_SLOGANS } from "@/lib/constants";
import styles from "./index.module.css";

function Background(): JSX.Element {
  return (
    <div className="pointer-events-none absolute left-0 top-0 h-full w-full overflow-hidden">
      <div
        className={cn(
          "absolute z-[-1] h-full w-full [--gradient-stop-1:60%] [--gradient-stop-2:85%] lg:[--gradient-stop-1:50%] lg:[--gradient-stop-2:90%]",
          "[--gradient-color-1=rgba(0,0,0,1)] [--gradient-color-2=rgba(0,0,0,0.8)] [--gradient-color-3=rgba(0,0,0,0)]",
          "dark:[--gradient-color-1=rgba(255,255,255,1)] dark:[--gradient-color-2=rgba(255,255,255,0.8)] dark:[--gradient-color-3=rgba(255,255,255,0)]"
        )}
        style={{
          background:
            "linear-gradient(180deg, var(--gradient-color-1) 0%, var(--gradient-color-2) var(--gradient-stop-1), var(--gradient-color-3) var(--gradient-stop-2), 100% transparent)",
        }}
      />
      <span className={cn(styles.leftLights, "opacity-50 dark:opacity-100")} />
      <span className={cn(styles.rightLights, "opacity-50 dark:opacity-100")} />
      <span className="absolute bottom-0 left-0 h-48 w-full bg-gradient-to-t from-white to-transparent dark:from-black" />
      <span className="absolute left-0 top-[20vh] h-[50vh] w-full bg-gradient-to-b from-white to-transparent dark:from-black" />
      <TurboheroBackground />
    </div>
  );
}

const variants = {
  hidden: { opacity: 0 },
  active: { opacity: 1 },
};

function Card({
  alt,
  href,
  title,
  icon: Icon,
  className,
  children,
}: {
  href: string;
  icon: React.ElementType;
  title: "repo" | "pack";
  alt?: string;
  className?: string;
  children: React.ReactNode;
}): JSX.Element {
  const [hovering, setHovering] = useState(false);
  return (
    <Link
      className={cn(
        styles["counter-border"],
        "h-[304]px w-[calc(100%_-_0px)] sm:h-[352px] sm:!w-[488px]"
      )}
      href={href}
      onMouseEnter={() => {
        setHovering(true);
      }}
      onMouseLeave={() => {
        setHovering(false);
      }}
    >
      <motion.i
        animate={hovering ? "active" : "hidden"}
        aria-hidden="true"
        initial="hidden"
        variants={variants}
      />
      <div
        className={cn(
          "relative flex h-full w-full max-w-full flex-col items-center justify-center overflow-hidden rounded-xl border border-[rgba(255,255,255,0.05)] p-3 !pb-12 pt-8 md:!pb-4 md:!pt-4",
          className
        )}
      >
        <div className="mb-7 flex flex-1 items-center justify-center md:mb-0">
          <Icon />
        </div>

        <div className="flex flex-1 flex-col items-center">
          {title === "pack" ? (
            <PackLogo
              alt={alt}
              className="mb-3 w-[160px] fill-black md:w-[220px] dark:fill-white"
            />
          ) : (
            <RepoLogo
              alt={alt}
              className="mb-3 w-[160px] fill-black md:w-[220px] dark:fill-white"
            />
          )}
          {children}
        </div>
      </div>
    </Link>
  );
}

function SiteCards(): JSX.Element {
  return (
    <div className="lg:!mt-15 container z-10 mt-8 flex w-full flex-col items-center justify-center gap-6 px-6 sm:mx-0 md:!mt-14 md:mb-0 lg:!translate-y-0 lg:!flex-row">
      <FadeIn delay={0.1}>
        <Card
          alt="Turborepo"
          className="turborepoCardBg"
          href="/repo"
          icon={Turborepo}
          title="repo"
        >
          <p className="!w-[280px] text-center font-mono text-lg opacity-50 md:!w-[340px] dark:opacity-70">
            The build system for JavaScript and TypeScript codebases.
          </p>
        </Card>
      </FadeIn>
      <FadeIn delay={0.2}>
        <Card
          alt="Turbopack"
          className="turbopackCardBg"
          href="/pack"
          icon={Turbopack}
          title="pack"
        >
          <p className="!w-[280px] text-center font-mono text-lg opacity-50 md:!w-[340px] dark:opacity-70 ">
            High-performance bundler for React Server Components and TypeScript
            codebases.
          </p>
        </Card>
      </FadeIn>
    </div>
  );
}

function Teams(): JSX.Element {
  return (
    <div className="mx-auto ">
      <p className="mb-2 bg-contain text-center text-sm font-semibold uppercase tracking-wide text-[#666666] md:!mb-4 dark:text-[#888888]">
        Trusted by teams from
        <br className="inline md:hidden" /> around the world
      </p>
      <div className="z-50 grid grid-flow-col grid-rows-6 sm:grid-rows-3 md:grid-rows-2 lg:grid-rows-1">
        <Clients
          companyList={[
            "Vercel",
            "AWS",
            "Microsoft",
            "Adobe",
            "Disney",
            "Netflix",
          ]}
          staticWidth
        />
      </div>
    </div>
  );
}

export default function LandingPage(): JSX.Element {
  return (
    <main className="pt-14 relative flex h-full w-full flex-col bg-white dark:bg-black items-center justify-center overflow-hidden [--geist-foreground:#fff] [--gradient-stop-1:0px] [--gradient-stop-2:120px] sm:[--gradient-stop-1:0px] sm:[--gradient-stop-2:120px] dark:[--geist-foreground:#000]">
      <Background />
      <FadeIn className="z-10 flex h-full w-full flex-col items-center justify-center">
        <h1 className="mx-6 mt-12 w-[300px] bg-gradient-to-b from-black/80 to-black bg-clip-text pb-4  text-center text-5xl font-extrabold leading-tight text-transparent md:!w-full lg:!mt-20 lg:text-6xl xl:leading-snug dark:from-white dark:to-[#AAAAAA]">
          Make Ship Happen
        </h1>
        <p className="mx-6 max-h-[112px] w-[315px] text-center font-mono text-xl text-[#666666] md:max-h-[96px] md:w-[700px] md:text-xl dark:text-[#888888]">
          {PRODUCT_SLOGANS.turbo}
        </p>
      </FadeIn>
      <SiteCards />
      <FadeIn className="z-10 py-16" delay={0.3}>
        <Teams />
      </FadeIn>
    </main>
  );
}
