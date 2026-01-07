import Image from "next/image";

export function Turborepo(): JSX.Element {
  return (
    <div className="relative h-24 w-24">
      <div className="pointer-events-none absolute left-1/2 top-1/2 h-[261px] w-[261px] -translate-x-1/2 -translate-y-1/2 bg-gradient-to-b from-[#FF3358] to-[#FF4FD8] opacity-5 mix-blend-normal blur-[60px] dark:opacity-[0.15]" />
      <div className="contents dark:hidden">
        <Image
          alt="Turborepo Logo"
          className="absolute left-1/2 top-1/2 w-[84px] -translate-x-1/2 -translate-y-1/2"
          height={120}
          src="/images/docs/repo/repo-hero-logo-light.svg"
          width={120}
        />
      </div>
      <div className="hidden dark:contents">
        <Image
          alt="Turborepo Logo"
          className="absolute left-1/2 top-1/2 hidden w-[84px] -translate-x-1/2 -translate-y-1/2 dark:block"
          height={120}
          src="/images/docs/repo/repo-hero-logo-dark.svg"
          width={120}
        />
      </div>
    </div>
  );
}
