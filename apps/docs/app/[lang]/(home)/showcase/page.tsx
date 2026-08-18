import { Clients } from "@/app/_clients/clients";
import { createMetadata } from "@/lib/create-metadata";

export const metadata = createMetadata({
  title: "Showcase",
  description: "Turborepo is the build system for coding agents.",
  canonicalPath: "/showcase",
});

function Showcase() {
  return (
    <main className="container mx-auto pt-12">
      <div className="mx-auto">
        <div className="py-16 lg:text-center">
          <h1 className="mb-6 mt-2 text-center text-heading-48 text-slate-900 dark:text-slate-100 lg:text-heading-56">
            Showcase
          </h1>
          <p className="text-center text-copy-20 text-gray-900">
            Who is using Turborepo?
          </p>
        </div>
      </div>

      <div className="mb-8 grid min-h-screen grid-cols-3 items-center gap-16 px-0 sm:grid-cols-4 sm:px-8 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-7 [&_img]:max-w-none">
        <Clients linked />
      </div>
    </main>
  );
}

export default Showcase;
