import type { ReactNode } from 'react';
export default function Layout({ children }: { children: ReactNode }) {
  return <html lang="zh-CN"><head><title>{"__TITLE__"}</title></head><body>{children}</body></html>;
}
