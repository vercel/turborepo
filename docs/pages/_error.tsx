import * as Sentry from "@sentry/nextjs";
import type { NextPage } from "next";
import type { ErrorProps } from "next/error";
import Error from "next/error";

// eslint-disable-next-line react/function-component-definition -- This is boilerplate code from https://docs.sentry.io/platforms/javascript/guides/nextjs/manual-setup/#react-render-errors-in-pages-router
const CustomErrorComponent: NextPage<ErrorProps> = (props) => {
  return <Error statusCode={props.statusCode} />;
};

CustomErrorComponent.getInitialProps = async (contextData) => {
  // In case this is running in a serverless function, await this in order to give Sentry
  // time to send the error before the lambda exits
  await Sentry.captureUnderscoreErrorException(contextData);

  // This will contain the status code of the response
  return Error.getInitialProps(contextData);
};

export default CustomErrorComponent;
