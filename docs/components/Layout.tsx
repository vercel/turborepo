import { Footer2 } from "./Footer2";
import { Header } from "./Header";
import { Sticky } from "./Sticky";

interface LayoutProps {
  preview?: boolean;
  dark?: boolean;
  stripe?: boolean;
  showCta: boolean;
  children: React.ReactNode;
}

export const Layout = ({
  preview,
  stripe = true,
  children,
  showCta = true,
  dark = true,
}: LayoutProps) => {
  return (
    <>
      <div
        className={"dark:bg-gray-900 dark:bg-opacity-50 bg-white min-h-screen"}
      >
        <Sticky>
          <Header />
        </Sticky>
        <main>{children}</main>
      </div>
      <Footer2 showCta={showCta} />
    </>
  );
};
