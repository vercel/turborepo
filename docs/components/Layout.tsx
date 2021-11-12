import { Footer2 } from "./Footer2";

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
      <main className={"dark:bg-dark dark:bg-opacity-50 bg-white"}>
        {children}
      </main>
    </>
  );
};
