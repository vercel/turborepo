import { cn } from "@/components/cn";
import {
  File as FumaFile,
  Folder as FumaFolder,
} from "fumadocs-ui/components/files";

export { Files } from "fumadocs-ui/components/files";

export function File({
  green,
  className,
  name,
  ...props
}: {
  green?: boolean;
  name: string;
  className: string;
}): JSX.Element {
  return (
    <FumaFile
      className={cn(
        green ? "text-green-700 dark:text-green-900" : "",
        className
      )}
      name={name}
      {...props}
    />
  );
}

export function Folder({
  green,
  className,
  name,
  ...props
}: {
  green?: boolean;
  name: string;
  className: string;
}): JSX.Element {
  return (
    <FumaFolder
      className={cn(
        green ? "text-green-700 dark:text-green-900" : "",
        className
      )}
      name={name}
      {...props}
    />
  );
}
