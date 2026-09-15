import type { CounterRequest } from './model';

export function increment(input: unknown): number {
  const value = (input as Partial<CounterRequest> | null)?.value;
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value >= Number.MAX_SAFE_INTEGER) {
    throw new Error('value 必须是可递增的安全整数');
  }
  return value + 1;
}
