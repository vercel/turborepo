import {
  File as FumaFile,
  Folder as FumaFolder
} from "fumadocs-ui/components/files";
import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

export { Files } from "fumadocs-ui/components/files";

type FileProps = ComponentProps<typeof FumaFile> & {
  green?: boolean;
};

export const File = ({ green, className, ...props }: FileProps) => (
  <FumaFile
    className={cn(green ? "text-green-700 dark:text-green-400" : "", className)}
    {...props}
  />
);

type FolderProps = ComponentProps<typeof FumaFolder> & {
  green?: boolean;
};

export const Folder = ({ green, className, ...props }: FolderProps) => (
  <FumaFolder
    className={cn(green ? "text-green-700 dark:text-green-400" : "", className)}
    {...props}
  />
);
