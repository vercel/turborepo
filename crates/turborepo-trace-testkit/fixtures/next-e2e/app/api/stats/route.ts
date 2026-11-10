import { readStats } from '../../../lib/stats';

export async function GET() {
  return Response.json(readStats());
}
