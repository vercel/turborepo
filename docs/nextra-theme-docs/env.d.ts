declare module "github-slugger" {
  export default class Slugger {
    slug(data: string): string;
  }
}

declare module "match-sorter";

declare module "title" {
  export default function title(
    title: string,
    special?: {
      special: string[];
    }
  );
}

declare module "@headlessui/react/dist/index.esm" {
  export * from "@headlessui/react/dist/index";
  export * from "@headlessui/react/dist/types";
}
