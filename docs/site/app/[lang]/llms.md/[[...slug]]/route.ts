import { notFound } from "next/navigation";
import { getLLMText, source } from "@/lib/geistdocs/source";
import { trackMdRequest } from "@/lib/md-tracking";

export const revalidate = false;

export async function GET(
  req: Request,
  { params }: RouteContext<"/[lang]/llms.md/[[...slug]]">
) {
  const { slug, lang } = await params;
  const page = source.getPage(slug, lang);

  if (!page) {
    notFound();
  }

  // Track markdown request (fire-and-forget)
  const userAgent = req.headers.get("user-agent");
  const referer = req.headers.get("referer");
  const acceptHeader = req.headers.get("accept");
  void trackMdRequest({
    path: `/llms.md/${slug?.join("/") ?? ""}`,
    userAgent,
    referer,
    acceptHeader
  });

  return new Response(await getLLMText(page), {
    headers: {
      "Content-Type": "text/markdown"
    }
  });
}

export const generateStaticParams = async ({
  params
}: RouteContext<"/[lang]/llms.md/[[...slug]]">) => {
  const { lang } = await params;

  return source.generateParams(lang);
};
