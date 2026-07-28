import { Clients } from "@/app/_clients/clients";
import { createMetadata } from "@/lib/create-metadata";

export const metadata = createMetadata({
  title: "Showcase",
  description:
    "Turborepo is a build system optimized for JavaScript and TypeScript, written in Rust.",
  canonicalPath: "/showcase"
});

function Showcase() {
  return (
    <main className="container mx-auto pt-12">
      <div className="mx-auto">
        <div className="py-16 lg:text-center">
          <h1 className="mt-2 text-3xl font-extrabold leading-8 tracking-tight text-black dark:text-white sm:text-4xl sm:leading-10 md:text-5xl">
            Showcase
          </h1>
          <p className="mt-4 max-w-3xl font-mono text-xl leading-7 text-black dark:text-white lg:mx-auto">
            Who is using Turborepo?
          </p>
        </div>
      </div>

      <div className="mb-8 px-0 min-h-screen sm:px-8 grid grid-cols-3 items-center gap-16 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-7 ">
        <Clients linked />
      </div>
    </main>
  );
}

export default Showcase;
