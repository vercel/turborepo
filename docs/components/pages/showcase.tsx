import { Clients } from "../clients/Clients";

export function Showcase() {
  return (
    <main className="relative">
      <div className="mx-auto">
        <div className="py-16 lg:text-center">
          <p className="text-base font-semibold leading-6 tracking-wide text-blue-600 uppercase dark:text-gray-400 font-space-grotesk">
            Showcase
          </p>
          <h1 className="mt-2 text-3xl font-extrabold leading-8 tracking-tight text-gray-900 md:text-5xl dark:text-white sm:text-4xl sm:leading-10">
            Who&apos;s using Turbo?
          </h1>
          <p className="max-w-3xl mt-4 text-xl leading-7 text-gray-500 dark:text-gray-400 lg:mx-auto font-space-grotesk">
            Turbo is the one of the fastest growing toolchains in the frontend
            ecosystem. It&apos;s trusted by thousands of developers in
            production including teams at Vercel, AWS, Netflix, Microsoft,
            Disney, and more.
          </p>
        </div>
      </div>

      <div className="grid items-center grid-cols-3 gap-16 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-7 ">
        <Clients linked />
      </div>
    </main>
  );
}
