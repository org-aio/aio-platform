import { increment } from '../../shared/counter/service';
import type { CounterResponse } from '../../shared/counter/model';

export function count(input: unknown, tenant: string | null): CounterResponse {
  return { value: increment(input), tenant_id: tenant || 'standalone' };
}
