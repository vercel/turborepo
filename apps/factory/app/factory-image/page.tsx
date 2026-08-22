import { readFactoryImageView } from "../../agent/lib/factory-image-registry";
import { FactoryImage } from "../factory-image";

export default async function FactoryImagePage() {
  return (
    <main id="main-content" className="pageContent">
      <h1 className="pageTitle">Factory image</h1>
      <FactoryImage initialView={await readFactoryImageView()} />
    </main>
  );
}
