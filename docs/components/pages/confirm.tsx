/* eslint-disable react/no-unescaped-entities */
import Head from "next/head";
import { Container } from "../Container";

export default function Confirm() {
  return (
    <>
      <Head>
        <title>Confirm</title>
        <meta name="robots" content="noindex" />
      </Head>
      <Container>
        <div className="container mx-auto">
          <div className="pt-20 mx-auto ">
            <div className="max-w-md mx-auto rounded-lg shadow-xl dark:bg-gray-900 dark:bg-opacity-80">
              <div className="p-6 rounded-lg shadow-sm ">
                <div className="mx-auto space-y-4 dark:text-white">
                  <h2 className="text-xl font-bold">Thanks so much!</h2>
                  <p>
                    Keep an eye on your inbox for product updates and
                    announcements from Turbo and Vercel.
                  </p>{" "}
                  <p>
                    Thanks,
                    <br />
                    The Turbo Team
                  </p>
                </div>
              </div>
            </div>
          </div>
        </div>
      </Container>
    </>
  );
}
