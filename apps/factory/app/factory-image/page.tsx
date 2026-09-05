import type { Metadata } from "next";

import { readFactoryImageView } from "../../agent/lib/factory-image-registry";
import { FactoryImage } from "../factory-image";

export const metadata: Metadata = {
  title: "Factory image"
};

export default async function FactoryImagePage() {
  return (
    <main
      id="main-content"
      className="mx-auto w-[min(1200px,calc(100%_-_48px))] max-[720px]:w-[min(1200px,calc(100%_-_32px))]"
    >
      <h1 className="mt-8 text-[clamp(1.5rem,3vw,2rem)] leading-tight font-semibold tracking-[-0.04em]">
        Factory image
      </h1>
      <FactoryImage initialView={await readFactoryImageView()} />
    </main>
  );
}
