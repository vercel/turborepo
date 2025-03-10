import classNames from "classnames";
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
      className={`${classNames({
        "text-green-600 dark:text-green-400": green,
      })} ${className}`}
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
      className={`${classNames({
        "text-green-600 dark:text-green-400": green,
      })} ${className}`}
      name={name}
      {...props}
    />
  );
}
