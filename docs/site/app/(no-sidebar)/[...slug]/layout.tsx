import { notFound } from "next/navigation";
import { extraPages } from "#app/source.ts";

export default async function SlugLayout(props: {
  params: Promise<{ slug?: Array<string> }>;
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
