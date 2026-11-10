export async function GET() {
  return Response.json({ ok: true });
}

export async function DELETE() {
  return new Response(null, { status: 204 });
}
