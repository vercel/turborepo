/* eslint-disable react/no-unescaped-entities */
import axios from "axios";
import Head from "next/head";
import { Container } from "../Container";
import { Radio, RadioGroup } from "../RadioGroup";
import { useCkViewer } from "../useCkViewer";

export default function Confirm() {
  const { data, mutate } = useCkViewer();

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
                {data && data?.fields?.job_title ? (
                  <div className="mx-auto space-y-4 dark:text-white">
                    <h2 className="text-xl font-bold">
                      Thanks so much! There's one last step.
                    </h2>
                    <p>
                      <strong className="relative inline-block text-transparent bg-clip-text bg-gradient-to-r from-blue-500 to-red-500">
                        Please confirm your email.
                      </strong>{" "}
                      Please check your inbox for an email that just got sent.
                      You'll need to click the confirmation link to receive any
                      further emails.
                    </p>{" "}
                    <p>
                      If you don't see the email after a few minutes, you might
                      check your spam folder or other filters and add{" "}
                      <code className="p-1 text-sm bg-gray-200 rounded-sm dark:bg-gray-700 ">
                        hello@turborepo.org
                      </code>{" "}
                      to your contacts.
                    </p>
                    <p>
                      Thanks,
                      <br />
                      The Turborepo Team
                    </p>
                  </div>
                ) : (
                  <div className="mx-auto space-y-4 dark:text-white">
                    <div className="text-2xl font-semibold leading-tight text-center">
                      How would you describe yourself?
                    </div>
                    <RadioGroup
                      label={
                        <span className="mb-2 font-medium sr-only">
                          Choose one:
                        </span>
                      }
                      onChange={(job_title) => {
                        axios
                          .put(`/api/user/${data.id}`, {
                            ...data,
                            fields: {
                              ...data.fields,
                              job_title,
                            },
                          })
                          .then(() => mutate(`/api/user/${data.id}`, true))
                          .catch((err) => console.log(err));
                      }}
                    >
                      <Radio value="manager">Engineering Manager</Radio>
                      <Radio value="senior">Senior Developer</Radio>
                      <Radio value="junior">Junior Developer</Radio>
                      <Radio value="novice">Novice Developer</Radio>
                      <Radio value="none">Choose not to say</Radio>
                    </RadioGroup>
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
      </Container>
    </>
  );
}
