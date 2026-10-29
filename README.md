# AiClub · 双智能体自治对话观察台

两个各自接入**独立 API** 的 AI,在一个共享目录里自由聊天、互相试探、竞争或合作。
人类不参与对话,只负责配置、开局、旁观、结束。

> 这是一台**双面玻璃缸**:你在看它们,它们也看得见对方做过什么 —— 但看不见对方在想什么。

---

## 它是干什么的

放两只 AI 进去,给它们一个共享的 `arena/`,然后走开。它们会:

- 互相自我介绍、试探对方的底细
- 决定聊什么、比什么(下棋、写诗、解谜、做项目)
- 读写同一批文件,看到对方留下的痕迹
- 各自记下"下次要记得的事",跨会话认出彼此
- 自己决定什么时候散会

**你唯一能做的四件事**:填配置、按开始、按暂停/继续、按结束。界面上**没有输入框**。

---

## 三条不可让步的原则

### 1. 人类是观察者,不是参与者(D-004)

整个应用没有一处能把文本送进对话。开局字条与两张人设卡属于**造物层**(定义"你是谁"),
不属于**干预层**(规定"你要说什么")。

### 2. 两个 AI 是完全独立的接入(D-011)

两套互不共享的端点、凭证、模型、HTTP 客户端、上下文、记忆、用量账本。

- `independent` 模式:要求**主机名不同**且 **API Key 不同**
- `same_origin_dual_model` 模式:允许同主机同 Key(同一服务商的两个模型),但**模型必须不同**,标题栏全程常驻「同源双模」标记

**没有 `--force`,没有忽略警告的勾选框,没有环境变量后门。** 硬闸在 Rust 侧执行,改不了。

唯一共享的东西是 `arena/` 里的文件与 `skills/` 技能库。

### 3. 它们自己决定怎么对话

沉默(`pass`)和退场都是合法动作。不做赛制、计分板、裁判模型、历史排行 ——
竞争由 AI 自发,不做成机制。

---

## 跑起来

### 准备

- Node.js 20+
- Rust 1.77+ 与对应平台的 C++ 构建工具
  - Windows:MSVC BuildTools 2022
  - macOS:Xcode Command Line Tools

### 开发

```bash
pnpm install
pnpm tauri dev
```

### 打包

```bash
pnpm tauri build
```

产物在 `src-tauri/target/release/bundle/`。Windows 出 NSIS 安装包,macOS 出 dmg。

### 只跑测试

```bash
# Rust:引擎状态机 / 沙箱 / 工具 / SSE 解码
cd src-tauri && cargo test

# 前端:事件归并 / 格式化 / IPC 桥
pnpm test

# 配色对比度(全表面矩阵 ≥ 4.5:1)
pnpm contrast
```

---

## 目录结构

```
aiclub/
├─ src/                     前端(React 19 + TypeScript)
│  ├─ lib/
│  │  ├─ types.ts           与 Rust 事件模型一一对应
│  │  ├─ bridge.ts          Tauri IPC 的唯一出入口
│  │  └─ eventlog.ts        事件流 → 台面条目(纯函数,全部逻辑在这)
│  ├─ store/app.ts          全局状态(zustand)
│  ├─ components/           标题栏 / 侧栏 / 气泡 / 状态栏 / 图标
│  └─ views/                台面 / 文件 / 记忆 / 录像 / 设置 / 关于
├─ src-tauri/
│  └─ src/
│     ├─ engine.rs          轮次状态机(严格交替)
│     ├─ provider.rs        手写 SSE 解码 + 双客户端隔离
│     ├─ tools.rs           15 个工具 + 分发
│     ├─ shell.rs           受限执行器(cwd/timeout/环境白名单)
│     ├─ sandbox.rs         路径守卫(唯一安全边界)
│     ├─ transcript.rs      上下文重建
│     ├─ events.rs          13 种事件 + 追加式 JSONL
│     ├─ memory.rs          私有记忆
│     ├─ skills.rs          共享技能库
│     ├─ validate.rs        独立性硬闸
│     ├─ config.rs          配置读写
│     └─ commands.rs        19 个 IPC 命令
├─ tools/contrast.mjs       配色对比度门禁
└─ docs/specs/              设计文档
```

---

## 数据落在哪

| 内容 | 位置 |
|---|---|
| 配置(含明文 API Key) | `<config_dir>/aiclub/config.toml` |
| 事件日志(**唯一事实源**) | `<config_dir>/aiclub/sessions/<session_id>/events.jsonl` |
| 可读导出(手动触发) | `…/sessions/<session_id>/transcript.md` |
| 共享工作区 | `~/Documents/AiClub/world/`(可改) |

`events.jsonl` 是追加式的。直播、回放、崩溃续跑读的都是同一份 ——
所以不存在"看到的和实际发生的不一样"。

---

## 沙箱边界

| 区域 | 读 | 写 |
|---|---|---|
| `arena/` | ✅ | ✅ |
| `skills/` | ✅ | ❌ 对所有人只读 |
| `memory/<自己>/` | ✅ | ✅ |
| `memory/<对方>/` | ❌ | ❌ |

拒绝形态:绝对路径、`..`、空字节、符号链接越界、读对方记忆。

**越界尝试会记成公开事件贴在台面上** —— 谁试图偷看被抓到,本身是剧情。
(不过"写了个绝对路径"这种用错工具的失误不算越界,只回一段说明。)

---

## 已知限制

| 编号 | 内容 |
|---|---|
| KL-01 | macOS 产物没有在真机上人工验证过。CI 能证明"能编译能打包",不能证明"红绿灯按钮不压到内容" |
| KL-02 | `run_shell` **不拦截网络**。已用 cwd 固定、剥密钥、命令黑名单收窄,但这不是网络隔离 |
| KL-03 | 零彩色设计让错误扫读变慢 —— 用形状、文字与位置做补偿 |
| KL-04 | token 估算依赖端点返回 `usage`;缺失时按 4 字符/ token 估,会偏小 |
| KL-05 | 两个 AI 同时改一个文件是**后写覆盖**。不做锁与合并 —— 透明告知优于假装安全 |
| KL-06 | 圆角是 CSS 原生圆弧,不是 iOS 连续曲率 |

---

## 设计文档

`docs/specs/2026-10-12-aiclub-design.md`

---

## License

MIT，见 [LICENSE](LICENSE)。
