/* eslint-disable react/no-unescaped-entities */
import { useTheme } from "next-themes";
import Head from "next/head";
import Image from "next/image";
import { users } from "../clients/users";
import { Container } from "../Container";

export default function Showcase() {
  const { theme } = useTheme();
  const showcase = users.map((user) => (
    <a
      href={user.infoLink}
      key={user.infoLink}
      target="_blank"
      rel="noopener noreferrer"
      className="flex items-center justify-center"
    >
      <Image
        src={user.image.replace(
          "/logos",
          theme == "dark" ? "/logos/white" : "/logos/color"
        )}
        alt={user.caption}
        width={user.style?.width ?? 100}
        height={user.style?.height ?? 75}
        loading="lazy"
        className="inline"
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
            <h1 className="mt-2 text-3xl font-extrabold leading-8 tracking-tight text-gray-900 dark:text-white sm:text-4xl sm:leading-10">
              Who's using Turborepo?
            </h1>
            <p className="max-w-4xl mt-4 text-xl leading-7 text-gray-500 lg:mx-auto">
              Turborepo is the one of the fastest growing build systems in the
              frontend ecosystem. It's trusted by thousands of developers in
              production including teams at Vercel, AWS, Netflix, Microsoft,
              Disney, and more.
            </p>
          </div>

          <div className="grid items-center grid-cols-3 gap-16 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 ">
            {showcase}
          </div>
        </div>
      </Container>
    </>
  );
}
