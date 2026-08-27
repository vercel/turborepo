import type { Metadata } from "next";
import { createExtraPage } from "../extra-page";

const termsPage = createExtraPage("terms");

export default termsPage.Page;

export const generateMetadata = async ({
  params
}: PageProps<"/[lang]/terms">): Promise<Metadata> => {
  const { lang } = await params;

  return termsPage.generateMetadata(lang);
};
