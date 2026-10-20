import type { Metadata } from "next";
import { createExtraPage } from "../extra-page";

const governancePage = createExtraPage("governance");

export default governancePage.Page;

export const generateMetadata = async ({
  params
}: PageProps<"/[lang]/governance">): Promise<Metadata> => {
  const { lang } = await params;

  return governancePage.generateMetadata(lang);
};
