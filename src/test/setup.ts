// Vitest 全局准备。
//
// ── 1. React 19 的 act() 标记 ──
// 没有这个全局标记时,每个组件测试都会刷
// "The current testing environment is not configured to support act(...)"。
// 它只关噪音,不影响断言。
(globalThis as unknown as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// ── 2. 补上 jsdom 环境下缺失的 localStorage ──
//
// 这不是"随便 mock 一下",是绕开一个具体的版本坑,所以把来龙去脉写清楚:
//
// · Node 22+ 把 Web Storage 做成了全局(localStorage / sessionStorage)。
//   本机是 Node 26,`'localStorage' in globalThis` 为 true,但**值为 undefined**
//   —— 除非启动时带 --experimental-webstorage / --localstorage-file。
// · vitest 的 jsdom 环境用 populateGlobal() 把 jsdom window 的属性搬进全局,
//   过滤规则是:`if (k in global) return keysArray.includes(k)`
//   —— 即"全局已有同名 key 时,只有它在白名单里才用 jsdom 的版本覆盖"。
// · 白名单(LIVING_KEYS)里有 sessionStorage,**没有 localStorage**。
//
// 两条凑在一起的结果:`sessionStorage` 是 jsdom 的真货,
// 而 `localStorage` 是 Node 留下的空壳(undefined)。
// 实测:typeof window.localStorage === "undefined" 而 sessionStorage === "object"。
//
// 影响:任何碰 localStorage 的测试都会以
// "Cannot read properties of undefined (reading 'clear')" 挂掉,
// 而那不是被测代码的错。
//
// 这里只在"确实缺"的时候补,且只补在 jsdom 环境(document 存在)下,
// 免得盖掉 Node 或浏览器提供的真实现。
if (typeof document !== "undefined" && typeof globalThis.localStorage === "undefined") {
  class MemoryStorage implements Storage {
    #m = new Map<string, string>();

    get length(): number {
      return this.#m.size;
    }
    clear(): void {
      this.#m.clear();
    }
    getItem(k: string): string | null {
      // Web Storage 的规矩:键会被转成字符串
      const key = String(k);
      return this.#m.has(key) ? (this.#m.get(key) as string) : null;
    }
    key(i: number): string | null {
      return [...this.#m.keys()][i] ?? null;
    }
    removeItem(k: string): void {
      this.#m.delete(String(k));
    }
    setItem(k: string, v: string): void {
      this.#m.set(String(k), String(v));
    }
  }

  Object.defineProperty(globalThis, "localStorage", {
    value: new MemoryStorage(),
    configurable: true,
    writable: true,
  });
}
