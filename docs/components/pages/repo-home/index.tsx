import Head from "next/head";
import { Toaster } from "react-hot-toast";
import { RepoHero } from "./RepoHero";
import { GradientSectionBorder } from "../home-shared/GradientSectionBorder";
import { RepoFeatures } from "./RepoFeatures";
import { RepoLetter } from "./RepoLetter";

export default function Home() {
  return (
    <>
      <Head>
        <title>Turborepo</title>
        <style>
          {`
            .dark body {
              background-color: black !important;
            }
          `}
        </style>
      </Head>
      <RepoHero />
      <GradientSectionBorder>
        <RepoFeatures />
      </GradientSectionBorder>
      <GradientSectionBorder>
        <RepoLetter />
      </GradientSectionBorder>
      <Toaster position="bottom-right" />
    </>
  );
}
