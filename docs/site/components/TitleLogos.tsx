import { VercelLogo } from "#app/_components/logos.tsx";
import { ThemeAwareImage } from "./theme-aware-image";

const size = 24;

export const TitleLogos = () => {
  return (
    <div className="my-auto ml-[4px] flex flex-row items-center justify-center">
      <VercelLogo />
      <svg
        className="ml-1 mr-1 text-[#eaeaea] dark:text-[#333]"
        fill="none"
        height={24}
        shapeRendering="geometricPrecision"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.5"
        viewBox="0 0 24 24"
      >
        <path d="M16.88 3.549L7.12 20.451" />
      </svg>

      <ThemeAwareImage
        light={{
          src: "/images/product-icons/repo-light-32x32.png",
          alt: "Turborepo logo",
          props: {
            src: "/images/product-icons/repo-light-32x32.png",
            alt: "Turborepo logo",
            width: size,
            height: size,
          },
        }}
        dark={{
          src: "/images/product-icons/repo-dark-32x32.png",
          alt: "Turborepo logo",
          props: {
            src: "/images/product-icons/repo-dark-32x32.png",
            alt: "Turborepo logo",
            width: size,
            height: size,
          },
        }}
      />
    </div>
  );
};
