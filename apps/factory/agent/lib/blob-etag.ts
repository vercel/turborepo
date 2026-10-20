/** Blob `get()` returns weak ETags, while conditional writes require strong ones. */
export function strongBlobEtag(etag: string): string {
  return etag.startsWith("W/") ? etag.slice(2) : etag;
}
