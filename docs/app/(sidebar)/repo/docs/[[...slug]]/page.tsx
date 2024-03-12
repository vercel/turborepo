import { notFound } from "next/navigation";
import { getPage } from "@/app/source";

export default function Page({ params }: { params: { slug?: string[] } }) {
  const page = getPage(params.slug);

  if (!page) {
    notFound();
  }

  const Mdx = page.data.exports.default;

  return (
    <>
      <h1 className="text-left">{page.data.title}</h1>
      <Mdx />
    </>
  );
}
