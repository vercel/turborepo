import setupFetch, { FetchError } from '@vercel/fetch';

export const runtime = 'nodejs';
export const dynamic = 'force-dynamic';

const fetch = setupFetch();

export const GET = (req) => {
  return Response.json({
    message: 'Hello from Next.js!',
    fetchError: FetchError, // Undefined
    fetchErrorType: typeof FetchError, // Undefined
    fetchErrorString: `${FetchError}`, // Undefined
  })
}

export const POST = (req) => {
  // This was the code I was trying to run
  // TypeError: Right-hand side of 'instanceof' is not an object
  (new Error()) instanceof FetchError;

  return Response.json({})
}
