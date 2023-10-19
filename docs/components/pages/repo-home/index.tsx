import { RepoHero } from "./RepoHero";
import { RepoFeatures } from "./RepoFeatures";
import { RepoLetter } from "./RepoLetter";
import { GradientSectionBorder } from "../home-shared/GradientSectionBorder";
import { LandingPageGlobalStyles } from "../home-shared/GlobalStyles";

export default function Home() {
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
