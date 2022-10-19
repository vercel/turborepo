import copy from "copy-to-clipboard";
import Head from "next/head";
import toast, { Toaster } from "react-hot-toast";
import { PackBenchmarks } from "./PackBenchmarks";
import { PackHero } from "./PackHero";
import { PackLetter } from "./PackLetter";
import { PackFeatures } from "./PackFeatures";
import { GradientSectionBorder } from "./GradientSectionBorder";

export default function Home() {
  return (
    <>
      <Head>
        <title>Turbopack</title>
      </Head>
      <PackHero />
      <GradientSectionBorder>
        <PackBenchmarks />
        <PackFeatures />
      </GradientSectionBorder>
      <GradientSectionBorder>
        <PackLetter />
      </GradientSectionBorder>
      <Toaster position="bottom-right" />
    </>
  );
}
