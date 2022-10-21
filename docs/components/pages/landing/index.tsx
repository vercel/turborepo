import Head from "next/head";
import { Clients } from "../../clients/Clients";
import { Marquee } from "../../clients/Marquee";
import { TurboheroBackground } from "./TurboHeroBackground";
import { Turborepo } from "./Turborepo";
import styles from "./index.module.css";
import { Turbopack } from "./Turbopack";
import React from "react";
import cn from "classnames";
import Link from "next/link";
import { AnimatePresence, motion } from "framer-motion";

function Background() {
  return (
    <div className="absolute top-0 left-0 w-full h-full overflow-hidden pointer-events-none">
      <div
        className={cn(
          "z-[-1] absolute w-full h-full [--gradient-color=0 0 0] [--gradient-stop-1:60%] [--gradient-stop-2:85%] lg:[--gradient-stop-1:50%] lg:[--gradient-stop-2:80%]",
          "[--gradient-color-1=rgba(0,0,0,1)] [--gradient-color-2=rgba(0,0,0,0.8)] [--gradient-color-3=rgba(0,0,0,0)]",
          "dark:[--gradient-color-1=rgba(255,255,255,1)] dark:[--gradient-color-2=rgba(255,255,255,0.8)] dark:[--gradient-color-3=rgba(255,255,255,0)]"
        )}
        style={{
          background:
            "linear-gradient(180deg, var(--gradient-color-1) 0%, var(--gradient-color-2) var(--gradient-stop-1), var(--gradient-color-3) var(--gradient-stop-2))",
        }}
      />
      <span className={styles.leftLights} />
      <span className={styles.rightLights} />
      <TurboheroBackground />
    </div>
  );
}

export function CardBadge({ children }: { children: React.ReactNode }) {
  return (
    <div className="font-mono font-bold text-xs text-black/50 dark:text-white/50  px-[6px] py-[3.25px] tracking-[-0.01em] rounded-[6px] uppercase flex justify-center items-center bg-black/5 dark:bg-white/[0.15] border border-black/[0.1] dark:border-white/[0.1]">
      {children}
    </div>
  );
}

const variants = {
  hidden: { opacity: 0 },
  active: { opacity: 1 },
};

function Card({
  href,
  title,
  icon: Icon,
  className,
  children,
}: {
  href: string;
  icon: React.ElementType;
  title: string;
  className?: string;
  children: React.ReactNode;
}) {
  const [hovering, setHovering] = React.useState(false);
  return (
    <Link href={href}>
      <a
        className={cn(
          styles["counter-border"],
          "w-[calc(100%_-_0px)] h-[304]px sm:!w-[488px] sm:h-[352px]"
        )}
        onMouseEnter={() => setHovering(true)}
        onMouseLeave={() => setHovering(false)}
      >
        <motion.i
          initial="hidden"
          animate={hovering ? "active" : "hidden"}
          variants={variants}
          aria-hidden="true"
        ></motion.i>
        <motion.div
          key="card-body"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ duration: 0.4, ease: [0.59, 0.15, 0.18, 0.93] }}
          className={cn(
            "relative w-full h-full max-w-full !pb-12 pt-8 md:!pb-4 md:!pt-4 p-3 rounded-xl overflow-hidden flex flex-col items-center justify-center border border-[rgba(255,255,255,0.05)]",
            className
          )}
        >
          <div className="mb-7">
            <Icon />
          </div>
          <p className="mb-3 text-xl font-bold tracking-wider uppercase font-wide md:text-3xl dark:text-white">
            {title}
          </p>
          {children}
        </motion.div>
      </a>
    </Link>
  );
}

function SiteCards() {
  return (
    <AnimatePresence>
      <div className="flex w-full container items-center justify-center gap-6 px-6 sm:mx-0 mt-8 md:!mt-14 lg:!mt-15 md:mb-0 flex-col lg:!flex-row z-10 lg:!translate-y-0">
        <Card
          title="Turborepo"
          icon={Turborepo}
          href="/repo"
          className="turborepoCardBg"
        >
          <p className="text-lg !w-[280px] md:!w-[340px] font-space-grotesk text-center opacity-50 dark:opacity-70">
            High-performance task runner for JavaScript and TypeScript
            codebases.
          </p>
        </Card>
        <Card
          title="Turbopack"
          icon={Turbopack}
          href="/pack"
          className="turbopackCardBg"
        >
          <div className="absolute top-3 left-3">
            <CardBadge>alpha</CardBadge>
          </div>
          <p className="text-lg !w-[280px] md:!w-[340px] font-space-grotesk text-center opacity-50 dark:opacity-70 ">
            The Rust-powered successor to Webpack.
          </p>
        </Card>
      </div>
    </AnimatePresence>
  );
}

function Teams() {
  return (
    <div className="mx-auto ">
      <p className="bg-contain mb-2 md:!mb-4 text-sm font-semibold tracking-wide text-center text-[#666666] dark:text-[#888888] uppercase">
        Trusted by teams from
        <br className="inline md:hidden" /> around the world
      </p>
      <Marquee>
        <Clients />
      </Marquee>
    </div>
  );
}

function LandingPage() {
  return (
    <>
      <Head>
        <title>Turbo</title>
        <meta
          name="og:description"
          content="Turbo is an incremental, distributed bundler and task runner optimized for JavaScript and TypeScript, written in Rust."
        />
      </Head>
      <div className="relative flex flex-col items-center justify-center w-full h-full  overflow-hidden [--geist-foreground:#fff] dark:[--geist-foreground:#000] [--gradient-stop-1:0px] [--gradient-stop-2:120px] sm:[--gradient-stop-1:0px] sm:[--gradient-stop-2:120px]">
        <Background />
        <div className="z-100 w-full h-full flex flex-col items-center justify-center">
          <h1 className="mt-12 lg:!mt-20 mx-6 w-[300px] md:!w-full font-extrabold text-5xl lg:text-6xl leading-tight text-center mb-4 bg-clip-text text-transparent bg-gradient-to-b from-black/80 to-black dark:from-white dark:to-[#AAAAAA]">
            Make Ship Happen
          </h1>
          <p className="mx-6 text-xl w-[315px] md:w-[615px] md:text-2xl font-space-grotesk text-center text-[#666666] dark:text-[#888888]">
            Turbo is an incremental, distributed bundler and task runner
            optimized for JavaScript and TypeScript, written in Rust.
          </p>
          <SiteCards />
        </div>
        <div className="z-10 py-16">
          <Teams />
        </div>
      </div>
    </>
  );
}

export default LandingPage;
