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
      /** 通过宿主原始下载响应启动整包下载，不受消息字节配额限制。 */
      downloadPackage(revision: string): Promise<Response>;
      /** 打开当前用户已授权的插件设置。 */
      openSettings(pageId: string): Promise<Response>;
      /** 唤起本机助手确认安装市场中已收录的指定版本。 */
      installTool(id: string, version: string): Promise<Response>;
      /** 读取已发布文档或公开 HTTPS 图片；宿主限制来源、类型和大小。 */
      managementImage(url: string): Promise<Response>;
      navigate(fragment: string, options?: { replace?: boolean }): Promise<string>;
      onNavigationChange(listener: (fragment: string) => void): () => void;
    };
  }
}
