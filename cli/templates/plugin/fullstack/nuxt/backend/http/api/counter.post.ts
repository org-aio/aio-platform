import { count } from '../../counter/service';
export default defineEventHandler(async event => {
  try { return count(await readBody(event), getHeader(event, 'x-aio-tenant-id') || null); }
  catch (error) { setResponseStatus(event, 400); return { error: String(error) }; }
});
