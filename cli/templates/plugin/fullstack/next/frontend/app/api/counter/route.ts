import { count } from '../../../../backend/counter/service';
export async function POST(request: Request) {
  try { return Response.json(count(await request.json(), request.headers.get('x-aio-tenant-id'))); }
  catch (error) { return Response.json({ error: String(error) }, { status: 400 }); }
}
