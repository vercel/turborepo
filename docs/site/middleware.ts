// This file gets overwritten during CI.
// We have it committed to source control like this
// so open source contributors can still run thing smoothly.

import { NextResponse } from "next/server";

export async function middleware(): Promise<Response> {
  return NextResponse.next();
}
