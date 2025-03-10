import type { Metadata } from "next/types";
import { Clients } from "@/app/_clients/clients";
import { PRODUCT_SLOGANS } from "@/lib/constants";
import { createMetadata } from "@/lib/create-metadata";

export function generateMetadata(): Metadata {
  return createMetadata({
    title: "Showcase",
    canonicalPath: "/showcase",
    description: PRODUCT_SLOGANS.turbo,
  });
}

function Showcase(): JSX.Element {
  return (
    <main className="container mx-auto pt-12">
      <div className="mx-auto">
        <div className="py-16 lg:text-center">
          <h1 className="mt-2 text-3xl font-extrabold leading-8 tracking-tight text-gray-900 dark:text-white sm:text-4xl sm:leading-10 md:text-5xl">
            Showcase
          </h1>
          <p className="mt-4 max-w-3xl font-mono text-xl leading-7 text-gray-500 dark:text-gray-400 lg:mx-auto">
            Who is using Turborepo?
          </p>
        </div>
      </div>

      <div className="mb-8 px-0 sm:px-8  grid grid-cols-3 items-center gap-16 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-7 ">
        <Clients linked />
      </div>
    </main>
  );
}

export default Showcase;
