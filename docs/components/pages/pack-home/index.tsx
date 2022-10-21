import Head from "next/head";

import { PackBenchmarks } from "./PackBenchmarks";
import { PackHero } from "./PackHero";
import { PackLetter } from "./PackLetter";
import { PackFeatures } from "./PackFeatures";
import { GradientSectionBorder } from "../home-shared/GradientSectionBorder";
import { LandingPageGlobalStyles } from "../home-shared/GlobalStyles";

export default function Home() {
  return (
    <>
      <Head>
        <title>Turbopack</title>
      </Head>
      <LandingPageGlobalStyles />
      <PackHero />
      <GradientSectionBorder>
        <PackBenchmarks />
        <PackFeatures />
      </GradientSectionBorder>
      <GradientSectionBorder>
        <PackLetter />
      </GradientSectionBorder>
    </>
  );
}
