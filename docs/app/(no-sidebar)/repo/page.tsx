import { GradientSectionBorder } from "../../_components/home-shared/GradientSectionBorder";
import { RepoHero } from "./components/RepoHero";
import { RepoFeatures } from "./components/RepoFeatures";
import { RepoLetter } from "./components/RepoLetter";

export default function TurborepoHome() {
  return (
    <>
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
