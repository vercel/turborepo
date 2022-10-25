/* eslint-disable react/no-unescaped-entities */
import { Container } from "../Container";
import { Clients } from "../clients/Clients";

export default function Showcase() {
  return (
    <main className="relative">
      <div className="mx-auto">
        <div className="py-16 lg:text-center">
          <p className="text-base font-semibold leading-6 tracking-wide text-blue-600 uppercase dark:text-gray-400 font-space-grotesk">
            Showcase
          </p>
          <h1 className="mt-2 text-3xl font-extrabold leading-8 tracking-tight text-gray-900 md:text-5xl dark:text-white sm:text-4xl sm:leading-10">
            Who's using Turbo?
          </h1>
          <p className="max-w-3xl mt-4 text-xl leading-7 text-gray-500 dark:text-gray-400 lg:mx-auto font-space-grotesk">
            Turbo is the one of the fastest growing toolchains in the frontend
            ecosystem. It's trusted by thousands of developers in production
            including teams at Vercel, AWS, Netflix, Microsoft, Disney, and
            more.
          </p>
        </div>
      </div>

      <div className="grid items-center grid-cols-3 gap-16 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-7 ">
        <Clients linked />
      </div>
      <Container>
        <div className="max-w-xl pt-20 pb-24 mx-auto space-y-6 text-center">
          <div className="mt-2 text-2xl font-extrabold leading-8 tracking-tight text-gray-900 dark:text-white sm:text-4xl sm:leading-10">
            Are you using Turbo?
          </div>
          <div className="mx-auto rounded-md">
            <a
              href="https://github.com/vercel/turbo/edit/main/docs/components/clients/users.ts"
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center justify-center w-auto px-8 py-3 text-base font-medium text-white no-underline bg-black border border-transparent rounded-md dark:bg-white dark:text-black betterhover:dark:hover:bg-gray-300 betterhover:hover:bg-gray-700 md:py-3 md:text-lg md:px-10 md:leading-6"
            >
              Add Your Company
            </a>
          </div>
        </div>
      </Container>
    </main>
  );
}
