import { GradientSectionBorder } from "../home-shared/GradientSectionBorder";
import { LandingPageGlobalStyles } from "../home-shared/GlobalStyles";
import { RepoHero } from "./RepoHero";
import { RepoFeatures } from "./RepoFeatures";
import { RepoLetter } from "./RepoLetter";

export function TurborepoHome() {
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
