import Image from "next/image";

export function Turbopack(): JSX.Element {
  return (
    <div className="relative h-24 w-24">
      <div className="pointer-events-none absolute left-1/2 top-1/2 h-[261px] w-[261px] -translate-x-1/2 -translate-y-1/2 bg-gradient-to-b from-[#4EBFFF] to-[#BD69FF] opacity-5 mix-blend-normal blur-[60px] dark:opacity-[0.15]" />
      <div className="contents dark:hidden">
        <Image
          alt=""
          className="absolute left-1/2 top-1/2 w-[84px] -translate-x-1/2 -translate-y-1/2"
          height={136.15}
          src="/images/docs/pack/turbopack-hero-logo-light.svg"
          width={120}
        />
      </div>
      <div className="hidden dark:contents">
        <Image
          alt=""
          className="absolute left-1/2 top-1/2 hidden w-[84px] -translate-x-1/2 -translate-y-1/2 dark:block"
          height={136.15}
          src="/images/docs/pack/turbopack-hero-logo-dark.svg"
          width={120}
        />
      </div>
    </div>
  );
}
