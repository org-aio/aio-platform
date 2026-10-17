export interface Header { name: string; value: string }
export interface Request { method?: string; path: string; query?: string; headers?: Header[]; body?: Uint8Array }
export interface Response { status: number; headers: Header[]; body: Uint8Array }
declare global {
  interface Window {
    readonly aioPlugin: {
      deviceView(input: { operation: "list" } | { operation: "open"; device: string; route?: string } | { operation: "close"; id: string }): Promise<unknown>;
      request(input: Request): Promise<Response>;
      /** 调用受部署信任和实时宿主权限约束的管理契约。 */
      manage(input: Request): Promise<Response>;
      json<T>(method: string, path: string, value?: unknown): Promise<T>;
      copy(text: string): Promise<Response>;
      download(name: string, body: Uint8Array, mime?: string): Promise<Response>;
      navigate(fragment: string, options?: { replace?: boolean }): Promise<string>;
      onNavigationChange(listener: (fragment: string) => void): () => void;
    };
  }
}
