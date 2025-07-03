import Image from "next/image";
import { Card } from "@repo/ui/card";
import { Gradient } from "@repo/ui/gradient";
import { TurborepoLogo } from "@repo/ui/turborepo-logo";

const LINKS = [
  {
    title: "Docs",
    href: "https://turborepo.com/docs",
    description: "Find in-depth information about Turborepo features and API.",
  },
  {
    title: "Learn",
    href: "https://turborepo.com/docs/handbook",
    description: "Learn more about monorepos with our handbook.",
  },
  {
    title: "Templates",
    href: "https://turborepo.com/docs/getting-started/from-example",
    description: "Choose from over 15 examples and deploy with a single click.",
  },
  {
    title: "Deploy",
    href: "https://vercel.com/new",
    description:
      "Instantly deploy your Turborepo to a shareable URL with Vercel.",
  },
];

export default function Page() {
  return (
    <main className="flex flex-col items-center justify-between min-h-screen p-24">
      <div className="z-10 items-center justify-between w-full max-w-5xl font-mono text-sm lg:flex">
        <p className="fixed top-0 left-0 flex justify-center w-full px-4 pt-8 pb-6 border backdrop-blur-2xl border-neutral-800 from-inherit lg:static lg:w-auto lg:rounded-xl lg:p-4">
          examples/with-tailwind -&nbsp;
          <code className="font-mono font-bold">web</code>
        </p>
        <div className="fixed bottom-0 left-0 flex items-end justify-center w-full h-48 lg:static lg:h-auto lg:w-auto">
          <a
            className="flex gap-2 p-8 pointer-events-none place-items-center lg:pointer-events-auto lg:p-0"
            href="https://vercel.com?utm_source=create-turbo&utm_medium=basic&utm_campaign=create-turbo"
            rel="noopener noreferrer"
            target="_blank"
          >
            By{" "}
            <Image
              alt="Vercel Logo"
              className="dark:invert"
              height={24}
              priority
              src="/vercel.svg"
              width={100}
            />
          </a>
        </div>
      </div>

      <div className="relative flex place-items-center ">
        <div className="font-sans w-auto pb-16 pt-[48px] md:pb-24 lg:pb-32 md:pt-16 lg:pt-20 flex justify-between gap-8 items-center flex-col relative z-0">
          <div className="z-50 flex items-center justify-center w-full">
            <div className="absolute min-w-[614px] min-h-[614px]">
              <Image
                alt="Turborepo"
                height={614}
                src="circles.svg"
                width={614}
              />
            </div>
            <div className="absolute z-50 flex items-center justify-center w-64 h-64">
              <Gradient
                className="opacity-90 w-[120px] h-[120px]"
                conic
                small
              />
            </div>

            <div className="flex justify-center items-center z-50">
              <TurborepoLogo />
            </div>
          </div>
          <Gradient
            className="top-[-500px] opacity-[0.15] w-[1000px] h-[1000px]"
            conic
          />
          <div className="z-50 flex flex-col items-center justify-center gap-5 px-6 text-center lg:gap-6">
            <svg
              className="w-[160px] md:w-[200px] fill-black dark:fill-white"
              viewBox="0 0 506 50"
              width={200}
              xmlns="http://www.w3.org/2000/svg"
            >
              <title>Turborepo logo</title>
              <path d="M53.7187 12.0038V1.05332H0.945312V12.0038H20.8673V48.4175H33.7968V12.0038H53.7187Z" />
              <path d="M83.5362 49.1431C99.764 49.1431 108.67 40.8972 108.67 27.3081V1.05332H95.7401V26.0547C95.7401 33.6409 91.7821 37.9287 83.5362 37.9287C75.2904 37.9287 71.3324 33.6409 71.3324 26.0547V1.05332H58.4029V27.3081C58.4029 40.8972 67.3084 49.1431 83.5362 49.1431Z" />
              <path d="M128.462 32.7174H141.325L151.484 48.4175H166.327L154.848 31.3321C161.313 29.0232 165.271 23.8778 165.271 16.8853C165.271 6.72646 157.685 1.05332 146.141 1.05332H115.532V48.4175H128.462V32.7174ZM128.462 22.4925V11.8719H145.481C150.033 11.8719 152.54 13.8509 152.54 17.2152C152.54 20.3816 150.033 22.4925 145.481 22.4925H128.462Z" />
              <path d="M171.287 48.4175H205.128C215.683 48.4175 221.752 43.404 221.752 35.0262C221.752 29.419 218.189 25.593 213.967 23.8778C216.87 22.4925 220.432 19.1942 220.432 13.9828C220.432 5.60502 214.495 1.05332 204.006 1.05332H171.287V48.4175ZM183.689 19.59V11.542H202.687C206.249 11.542 208.228 12.9273 208.228 15.566C208.228 18.2047 206.249 19.59 202.687 19.59H183.689ZM183.689 29.2871H203.875C207.371 29.2871 209.284 31.0022 209.284 33.5749C209.284 36.1476 207.371 37.8628 203.875 37.8628H183.689V29.2871Z" />
              <path d="M253.364 0.261719C236.806 0.261719 224.866 10.6185 224.866 24.7354C224.866 38.8523 236.806 49.2091 253.364 49.2091C269.922 49.2091 281.796 38.8523 281.796 24.7354C281.796 10.6185 269.922 0.261719 253.364 0.261719ZM253.364 11.4761C262.072 11.4761 268.602 16.6215 268.602 24.7354C268.602 32.8493 262.072 37.9947 253.364 37.9947C244.656 37.9947 238.126 32.8493 238.126 24.7354C238.126 16.6215 244.656 11.4761 253.364 11.4761Z" />
              <path d="M300.429 32.7174H313.292L323.451 48.4175H338.294L326.815 31.3321C333.28 29.0232 337.238 23.8778 337.238 16.8853C337.238 6.72646 329.652 1.05332 318.108 1.05332H287.499V48.4175H300.429V32.7174ZM300.429 22.4925V11.8719H317.448C322 11.8719 324.507 13.8509 324.507 17.2152C324.507 20.3816 322 22.4925 317.448 22.4925H300.429Z" />
              <path d="M343.254 1.05332V48.4175H389.299V37.467H355.92V29.7489H385.539V19.0622H355.92V12.0038H389.299V1.05332H343.254Z" />
              <path d="M408.46 33.3111H425.677C437.221 33.3111 444.807 27.7699 444.807 17.2152C444.807 6.59453 437.221 1.05332 425.677 1.05332H395.53V48.4175H408.46V33.3111ZM408.46 22.5585V11.8719H424.951C429.569 11.8719 432.076 13.8509 432.076 17.2152C432.076 20.5135 429.569 22.5585 424.951 22.5585H408.46Z" />
              <path d="M476.899 0.261719C460.341 0.261719 448.401 10.6185 448.401 24.7354C448.401 38.8523 460.341 49.2091 476.899 49.2091C493.456 49.2091 505.33 38.8523 505.33 24.7354C505.33 10.6185 493.456 0.261719 476.899 0.261719ZM476.899 11.4761C485.606 11.4761 492.137 16.6215 492.137 24.7354C492.137 32.8493 485.606 37.9947 476.899 37.9947C468.191 37.9947 461.66 32.8493 461.66 24.7354C461.66 16.6215 468.191 11.4761 476.899 11.4761Z" />
            </svg>
          </div>
        </div>
      </div>

      <div className="grid mb-32 text-center lg:max-w-5xl lg:w-full lg:mb-0 lg:grid-cols-4 lg:text-left">
        {LINKS.map(({ title, href, description }) => (
          <Card href={href} key={title} title={title}>
            {description}
          </Card>
        ))}
      </div>
    </main>
  );
}
