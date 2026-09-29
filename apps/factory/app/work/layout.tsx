import type { ReactNode } from "react";

import { ModelPickerEnhancer } from "../model-picker";

interface WorkLayoutProps {
  readonly children: ReactNode;
}

export default function WorkLayout({ children }: WorkLayoutProps) {
  return (
    <>
      {children}
      <ModelPickerEnhancer />
    </>
  );
}
