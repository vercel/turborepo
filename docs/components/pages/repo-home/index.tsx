import Head from "next/head";

import { RepoHero } from "./RepoHero";
import { RepoFeatures } from "./RepoFeatures";
import { RepoLetter } from "./RepoLetter";
import { GradientSectionBorder } from "../home-shared/GradientSectionBorder";
import { LandingPageGlobalStyles } from "../home-shared/GlobalStyles";

export default function Home() {
  return (
    <>
      <Head>
        <title>Turborepo</title>
      </Head>
      <LandingPageGlobalStyles />
      <RepoHero />
      <GradientSectionBorder>
        <RepoFeatures />
      </GradientSectionBorder>
      <GradientSectionBorder>
        <RepoLetter />
      </GradientSectionBorder>
    </>
  );
}
