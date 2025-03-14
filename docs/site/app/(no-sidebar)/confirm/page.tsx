export function Confirm(): JSX.Element {
  return (
    <div className="container mx-auto">
      <div className="mx-auto py-20">
        <div className="mx-auto max-w-md rounded-lg shadow-xl">
          <div className="rounded-lg p-6 shadow-sm ">
            <div className="mx-auto space-y-4 dark:text-white">
              <h2 className="text-xl font-bold">Thanks so much!</h2>
              <p>
                Keep an eye on your inbox for product updates and announcements
                from Turborepo and Vercel.
              </p>{" "}
              <p>
                Thanks,
                <br />
                The Turborepo Team
              </p>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
export default Confirm;
