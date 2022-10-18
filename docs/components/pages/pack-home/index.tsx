import copy from "copy-to-clipboard";
import Head from "next/head";
import toast, { Toaster } from "react-hot-toast";
import { PackBenchmarks } from "./PackBenchmarks";
import { PackHero } from "./PackHero";
import { PackLetter } from "./PackLetter";
import { PackFeatures } from "./PackFeatures";
import { GradientSectionBorder } from "./GradientSectionBorder";

export default function Home() {
  const onClick = () => {
    copy("npx create-turbo@latest");
    toast.success("Copied to clipboard");
  };

  return (
    <>
      <Head>
        <title>Turbopack</title>
      </Head>
      <PackHero />
      <GradientSectionBorder hexBottomOffset={560}>
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
