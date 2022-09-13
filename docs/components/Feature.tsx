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
      <div className="bg-circle-pink"></div>
      <div className="bg-circle-blue"></div>
      <div className="inline-flex items-center space-x-3">
        <div className="flex items-center justify-center bg-black rounded-full bg-opacity-5 w-9 h-9 icon-circle">
          <Icon
            className="h-8 w-8 dark:text-white flex-shrink-0 rounded-full p-1.5 text-black block dark:stroke-[url(#pink-gradient)]"
            aria-hidden="true"
          />
        </div>
        <h3 className="m-0 text-lg font-semibold leading-6 tracking-tighter text-gray-900 dark:text-white">
          {name}
        </h3>
      </div>
      <div>
        <p className="mt-2 text-base font-medium leading-7 text-gray-500 dark:text-gray-400">
          {description}
        </p>
      </div>
      <style jsx global>{`
        html.dark .icon-circle {
          background: linear-gradient(180deg, rgba(50, 134, 241, 0.2) 0%, rgba(195, 58, 195, 0.2) 100%);
        }
        .bg-circle-blue {
          position: absolute;
          width: 180px;
          height: 180px;
          top: 0;
          left: calc(25% - 180px / 2); 
          top: -90px;
          z-index: -1;         

          mix-blend-mode: normal;
          filter: blur(90px);
          opacity: 0.3;
          background: linear-gradient(180deg, #4EBFFF 0%, #BD69FF 100%);
        }

        .bg-circle-pink {
          position: absolute;
          width: 180px;
          height: 180px;
          top: 0;
          left: calc(75% - 180px / 2); 
          top: -90px;
          z-index: -1;         

          background: linear-gradient(180deg, #FF3358 0%, #FF4FD8 100%);
          mix-blend-mode: normal;
          filter: blur(90px);
          opacity: 0.3;
        }
      `}</style>
    </>
  );
};

const featureWrapperClasses = `relative block overflow-hidden p-10 bg-white shadow-lg rounded-xl dark:bg-opacity-5 no-underline text-black dark:text-white`;

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
