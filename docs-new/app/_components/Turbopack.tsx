import Image from "next/image";

export function Turbopack() {
  return (
    <div className="relative w-24 h-24">
      <div className="pointer-events-none absolute w-[261px] h-[261px] top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 bg-gradient-to-b from-[#4EBFFF] to-[#BD69FF] mix-blend-normal opacity-5 dark:opacity-[0.15] blur-[60px]" />
      <div className="contents dark:hidden">
        <Image
          alt=""
          className="absolute w-[84px] top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2"
          height={136.15}
          src="/images/docs/pack/turbopack-hero-logo-light.svg"
          width={120}
        />
      </div>
      <div className="dark:contents hidden">
        <Image
          alt=""
          className="hidden dark:block absolute w-[84px] top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2"
          height={136.15}
          src="/images/docs/pack/turbopack-hero-logo-dark.svg"
          width={120}
        />
      </div>
    </div>
  );
}
