import { notFound } from "next/navigation";
import { extraPages } from "@/app/source";

export default async function SlugLayout(props: {
  params: Promise<{ slug?: string[] }>;
  children: React.ReactNode;
}): Promise<JSX.Element> {
  const params = await props.params;

  const { children } = props;

  const page = extraPages.getPage(params.slug);

  if (!page) {
    notFound();
  }

  return <>{children}</>;
}
