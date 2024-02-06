import { GradientSectionBorder } from "../../_components/home-shared/GradientSectionBorder";
import { LandingPageGlobalStyles } from "../../_components/home-shared/GlobalStyles";
import { RepoHero } from "./components/RepoHero";
import { RepoFeatures } from "./components/RepoFeatures";
import { RepoLetter } from "./components/RepoLetter";

export default function TurborepoHome() {
  return (
    <>
      <LandingPageGlobalStyles />
      <main className="relative">
        <RepoHero />
        <GradientSectionBorder>
          <RepoFeatures />
        </GradientSectionBorder>
        <GradientSectionBorder>
          <RepoLetter />
        </GradientSectionBorder>
      </main>
    </>
  );
}
