import Image from "next/image";

export function Turborepo() {
  return (
    <div className="relative w-24 h-24">
      <div className="pointer-events-none absolute w-[261px] h-[261px] top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 bg-gradient-to-b from-[#FF3358] to-[#FF4FD8] mix-blend-normal opacity-5 dark:opacity-[0.15] blur-[60px]" />
      <div className="contents dark:hidden">
        <Image
          alt="Turborepo Logo"
          src={`/images/docs/repo/repo-hero-logo-light.svg`}
          width={120}
          height={120}
          className="absolute w-[84px] top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2"
        />
      </div>
      <div className="dark:contents hidden">
        <Image
          alt="Turborepo Logo"
          src={`/images/docs/repo/repo-hero-logo-dark.svg`}
          width={120}
          height={120}
          className="hidden dark:block absolute w-[84px] top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2"
        />
      </div>
    </div>
  );
}
