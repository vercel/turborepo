import type { Metadata } from "next";
import { Clients } from "@/app/_clients/clients";
import { createMetadata } from "@/lib/create-metadata";
import { getLocalizedPath } from "@/lib/geistdocs/public-path";

export const generateMetadata = async ({
  params
}: PageProps<"/[lang]/showcase">): Promise<Metadata> => {
  const { lang } = await params;

  return createMetadata({
    title: "Showcase",
    description: "Turborepo is the build system for coding agents.",
    canonicalPath: getLocalizedPath(lang, "/showcase")
  });
};

function Showcase() {
  return (
    <div className="container mx-auto pt-12">
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

      <div className="mb-8 grid min-h-screen grid-cols-2 items-center gap-6 sm:gap-16 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-5 xl:grid-cols-7">
        <Clients linked />
      </div>
    </div>
  );
}

export default Showcase;
