/* eslint-disable react/no-unescaped-entities */
import { useTheme } from "next-themes";
import Head from "next/head";
import Image from "next/future/image";
import { users } from "../clients/users";
import { Container } from "../Container";

export default function Showcase() {
  const { theme } = useTheme();

  const showcase = users
    .filter((p) => p.pinned)
    .map((user, index) => (
      <a
        href={user.infoLink}
        key={`${user.infoLink}-${theme}-${index}-light`}
        target="_blank"
        rel="noopener noreferrer"
        className="flex justify-center item-center dark:hidden"
      >
        <Image
          src={user.image.replace("/logos", "/logos/color")}
          alt={user.caption}
          width={user.style?.width ?? 100}
          height={user.style?.height ?? 75}
          priority={true}
          className="inline w-auto"
        />
      </a>
    ));

  const showcaseLight = users
    .filter((p) => p.pinned)
    .map((user, index) => (
      <a
        href={user.infoLink}
        key={`${user.infoLink}-${theme}-${index}-dark`}
        target="_blank"
        rel="noopener noreferrer"
        className="justify-center hidden item-center dark:flex"
      >
        <Image
          key={`${user.infoLink}-${theme}-${index}-dark`}
          src={user.image.replace("/logos", "/logos/white")}
          alt={user.caption}
          width={user.style?.width ?? 100}
          height={user.style?.height ?? 75}
          priority={true}
          className="inline w-auto"
        />
      </a>
    ));
  return (
    <>
      <Head>
        <title>Showcase</title>
      </Head>
      <Container>
        <div className="container mx-auto">
          <div className="py-16 lg:text-center">
            <p className="text-base font-semibold leading-6 tracking-wide text-blue-600 uppercase dark:text-gray-400">
              Showcase
            </p>
            <h1 className="mt-2 text-3xl font-extrabold leading-8 tracking-tight text-gray-900 md:text-5xl dark:text-white sm:text-4xl sm:leading-10">
              Who's using Turborepo?
            </h1>
            <p className="max-w-3xl mt-4 text-xl leading-7 text-gray-500 lg:mx-auto">
              Turborepo is the one of the fastest growing build systems in the
              frontend ecosystem. It's trusted by thousands of developers in
              production including teams at Vercel, AWS, Netflix, Microsoft,
              Disney, and more.
            </p>
          </div>

          <div className="grid items-center grid-cols-3 gap-16 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 ">
            {showcase}
            {showcaseLight}
          </div>
          <div className="max-w-xl pt-20 pb-24 mx-auto space-y-6 text-center">
            <div className="mt-2 text-2xl font-extrabold leading-8 tracking-tight text-gray-900 dark:text-white sm:text-4xl sm:leading-10">
              Are you using Turborepo?
            </div>
            <div className="mx-auto rounded-md">
              <a
                href="https://github.com/vercel/turborepo/edit/main/docs/components/clients/users.ts"
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center justify-center w-auto px-8 py-3 text-base font-medium text-white no-underline bg-black border border-transparent rounded-md dark:bg-white dark:text-black betterhover:dark:hover:bg-gray-300 betterhover:hover:bg-gray-700 md:py-3 md:text-lg md:px-10 md:leading-6"
              >
                Add Your Company
              </a>
            </div>
          </div>
        </div>
      </Container>
    </>
  );
}
