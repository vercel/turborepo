import Link from "next/link";
import type { Feature } from "../content/features";

type FeatureProps = {
  feature: Omit<Feature, "page">;
  // include feature description
  detailed?: boolean;
};

const DetailedFeatureInner = (props: { feature: FeatureProps["feature"] }) => {
  const { Icon, name, description } = props.feature;
  return (
    <>
      <div className="inline-flex items-center space-x-3">
        <Icon
          className="h-8 w-8 dark:text-white flex-shrink-0 rounded-full p-1.5 dark:bg-white dark:bg-opacity-10 bg-black bg-opacity-5 text-black block"
          aria-hidden="true"
        />
        <h3 className="m-0 text-lg font-medium leading-6 dark:text-white">
          {name}
        </h3>
      </div>
      <div>
        <p className="mt-2 text-base font-medium leading-7 text-gray-500 dark:text-gray-400">
          {description}
        </p>
      </div>
    </>
  );
};

const featureWrapperClasses = `block p-10 bg-white shadow-lg rounded-xl dark:bg-opacity-5 no-underline text-black dark:text-white`;

export const DetailedFeatureLink = (props: {
  href: string;
  feature: FeatureProps["feature"];
}) => {
  return (
    <Link href={props.href}>
      <a className={featureWrapperClasses}>
        <DetailedFeatureInner feature={props.feature}></DetailedFeatureInner>
      </a>
    </Link>
  );
};

export default function Feature(props: FeatureProps) {
  const { feature, detailed = false } = props;
  const { Icon, name, description } = feature;

  if (detailed) {
    return (
      <div className={featureWrapperClasses}>
        <DetailedFeatureInner feature={feature} />
      </div>
    );
  }

  return (
    <div className="flex items-center space-x-4">
      <div>
        <Icon
          className="block w-8 h-8 text-black dark:text-white"
          style={{ height: 24, width: 24 }}
          aria-hidden="true"
        />
      </div>
      <div>
        <div className="my-0 font-medium dark:text-white">{name}</div>
      </div>
    </div>
  );
}
