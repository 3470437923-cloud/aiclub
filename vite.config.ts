import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 开发服务器固定端口;清屏交给 Tauri,别刷掉它的日志。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5190,
    strictPort: true,
    host: "127.0.0.1",
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "chrome110",
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
    // 组件测试用文件头的 @vitest-environment jsdom 单独声明,默认仍是 node
    setupFiles: ["src/test/setup.ts"],
    // 侧栏滚动契约测试要读 surface.css 的真实内容(?raw)。
    // vitest 默认把 CSS 模块 mock 成空串,会把那个测试变成永远绿的假测试。
    css: true,
    // jsdom 默认 url 是 about:blank,那是 opaque origin —— 此时
    // window.localStorage 直接不存在(不是空对象,是 undefined)。
    // 主题测试要读写 localStorage,所以必须给一个真实 origin。
    environmentOptions: { jsdom: { url: "http://localhost/" } },
  },
});
