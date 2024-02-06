import { GradientSectionBorder } from "../home-shared/GradientSectionBorder";
import { LandingPageGlobalStyles } from "../home-shared/GlobalStyles";
import { PackHero } from "./PackHero";
import { PackLetter } from "./PackLetter";
import { PackFeatures } from "./PackFeatures";

export function TurbopackHome() {
  return (
    <>
      <LandingPageGlobalStyles />
      <main className="relative">
        <PackHero />
        <GradientSectionBorder>
          <PackFeatures />
        </GradientSectionBorder>
        <GradientSectionBorder>
          <PackLetter />
        </GradientSectionBorder>
      </main>
    </>
  );
}
