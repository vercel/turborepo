import { DuplicateIcon } from "@heroicons/react/outline";
import copy from "copy-to-clipboard";
import Head from "next/head";
import Image from "next/future/image";
import Link from "next/link";
import toast, { Toaster } from "react-hot-toast";
import edelman from "../../images/edelman.jpeg";
import elad from "../../images/elad.jpeg";
import flavio from "../../images/flavio.jpeg";
import jongold from "../../images/jongold.jpeg";
import ollermi from "../../images/ollermi.jpeg";
import shadcn from "../../images/shadcn.jpeg";
import christian from "../../images/christian.jpeg";
import yangshunz from "../../images/yangshunz.jpeg";
import nmoore from "../../images/nmoore.jpeg";
import joshlarson from "../../images/joshlarson.jpeg";
import paularmstrong from "../../images/paularmstrong.jpeg";

import { Container } from "../Container";
import Tweet, { Mention } from "../Tweet";
import { HomeFeatures } from "../Features";
import { Marquee } from "../clients/Marquee";
import { Clients } from "../clients/Clients";

export default function Home() {
  const onClick = () => {
    copy("npx create-turbo@latest");
    toast.success("Copied to clipboard");
  };

  return (
    <>
      <Head>
        <title>Turborepo</title>
        <meta
          name="og:description"
          content="Turborepo is a high-performance build system for JavaScript and
          TypeScript codebases"
        />
      </Head>
      <div className="w-auto px-4 pt-16 pb-8 mx-auto sm:pt-24 lg:px-8">
        <h1 className="max-w-5xl text-center mx-auto text-6xl font-extrabold tracking-tighter leading-[1.1] sm:text-7xl lg:text-8xl xl:text-8xl">
          <span className="inline-block text-transparent bg-clip-text bg-gradient-to-r from-pink-gradient-start to-blue-500 ">
            Make ship happen.
          </span>{" "}
        </h1>
        <p className="max-w-lg mx-auto mt-6 font-medium leading-tight text-center text-gray-400 sm:max-w-4xl sm:text-2xl md:text-3xl ">
          Turbo is an incremental, distributed bundler and task runner optimized
          for JavaScript and TypeScript.
        </p>
        <div className="max-w-xl mx-auto mt-5 sm:flex sm:justify-center md:mt-8">
          <div className="rounded-md ">
            <Link href="/repo">
              <a className="flex items-center justify-center w-full px-8 py-3 text-base font-medium text-white no-underline bg-black border border-transparent rounded-md dark:bg-white dark:text-black betterhover:dark:hover:bg-gray-300 betterhover:hover:bg-gray-700 md:py-3 md:text-lg md:px-10 md:leading-6">
                Turborepo
              </a>
            </Link>
          </div>
          <div className="rounded-md ">
            <Link href="/pack">
              <a className="flex items-center justify-center w-full px-8 py-3 text-base font-medium text-white no-underline bg-black border border-transparent rounded-md dark:bg-white dark:text-black betterhover:dark:hover:bg-gray-300 betterhover:hover:bg-gray-700 md:py-3 md:text-lg md:px-10 md:leading-6">
                Turbopack
              </a>
            </Link>
          </div>
        </div>
      </div>

      <div className="py-16">
        <div className="mx-auto ">
          <p className="pb-8 text-sm font-semibold tracking-wide text-center text-gray-400 uppercase dark:text-gray-500">
            Trusted by teams from around the world
          </p>
          <Marquee>
            <Clients />
          </Marquee>
        </div>
      </div>
      <Toaster position="bottom-right" />
    </>
  );
}
