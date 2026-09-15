import type { CounterRequest, CounterResponse } from './model';

declare global {
  interface Window {
    aioPlugin?: { json<T>(method: string, path: string, value?: unknown): Promise<T> };
  }
}

export async function incrementOnServer(value: number): Promise<CounterResponse> {
  const input: CounterRequest = { value };
  if (window.aioPlugin) return window.aioPlugin.json<CounterResponse>('POST', '/api/counter', input);
  const response = await fetch('/api/counter', {
    method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(input),
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || `HTTP ${response.status}`);
  return result;
}
