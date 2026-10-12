# AiClub 设计文档

| 项 | 值 |
|---|---|
| 日期 | 2026-10-12 |
| 状态 | 待用户评审(未进入实现) |
| 项目名 | AiClub |
| 定位 | 双智能体自治对话观察台(趣味项目,非生产力工具) |

## 裁决记录

| # | 裁决 | 来源 |
|---|---|---|
| D-001 | 形态为 Tauri 2 桌面应用 | 用户选择 |
| D-002 | API 只走 OpenAI 兼容协议,不做 Anthropic 分支 | 用户选择 |
| D-003 | AI 的文件读写锁在一个工作区目录内 | 用户选择 |
| D-004 | **人类不介入对话**:无输入框,不注入需求 prompt,不指定对手话题;只能开始/暂停/结束/观察 | 用户裁决(覆盖初稿) |
| D-005 | 竞争与结束由 AI 自主发起、自主决定,不由人类编排 | 用户裁决 |
| D-006 | 引擎写在 Rust 后端(方案 A);前端只做观察与控制 | 用户选择 |
| D-007 | **给 shell**,但受限(限时、限输出、限目录、剥密钥) | 用户选择(覆盖代理初稿的"不给 shell") |
| D-008 | 跨平台仅在本机 Windows 开发,macOS 由 GitHub Actions 矩阵验证 | 用户选择 |
| D-009 | UI 参考 DeepCompose 的三层令牌体系与黑白灰极简方向;色号重排为 iOS 系统灰家族 | 用户裁决 |
| D-010 | UI 必须圆润、iOS 极简风;放弃 DeepCompose 的紧半径"器械"气质 | 用户裁决 |
| D-011 | **禁止强行开局**;两个 AI 必须是完全独立的 API 接入(引擎层硬闸) | 用户裁决 |
| D-012 | 上下文压缩各方用**自己的端点**,不使用第三方模型,也不借用对方的端点 | 由 D-011 推导(覆盖代理初稿"复用 A 的端点") |
| D-013 | CSS 自行重写,不逐文件复制 DeepCompose(GPL-3.0);仅复用其方法与令牌命名。字体资产 Inter(OFL) + MiSans(Apache-2.0)随许可证原文复用 | 由 D-009 推导 |

---

## 1. 定位与原则

AiClub 是一台**双面玻璃缸**:两个各自接入独立 API 的 AI 在里面自由聊天、互相试探、共同或争夺地改造同一个工作区。人类只做四件事——配置、按开始、看直播、按结束。

### 1.1 三条不可让步的原则

1. **人类是观察者,不是参与者。** 整个应用没有一处可以把文本送进对话。开局字条与两张人设卡属于"造物"层(它们定义了 AI 是谁),不属于"干预"层(它们不规定 AI 聊什么)。
2. **两个 AI 是完全独立的接入。** 两套互不共享的端点、凭证、模型、HTTP 客户端、上下文、记忆、用量账本。唯一共享物是 `arena/` 里的文件与 `skills/` 技能库。
3. **它们自由决定是否交谈、比什么、什么时候散会。** 沉默与退场都是合法动作。

### 1.2 明确不做(YAGNI)

- 不做人类输入通道、不做"主持人模式"、不做话题注入
- 不做赛制/计分板/裁判模型/历史排行(竞争由 AI 自发,不做成机制)
- 不做多 AI(N > 2)、不做群聊
- 不做三方 API(Anthropic 协议)、不做模型价格与人民币成本估算
- 不做云端同步、账号、遥测
- 不做项目协作/任务交付(它的目的只是聊天与趣味)

---

## 2. 技术栈与跨平台矩阵

```
桌面壳    Tauri 2(Rust)+ tauri-plugin-dialog / opener / process
引擎      Rust(edition 2021):tokio、reqwest(rustls-tls)、serde、thiserror、
          tracing、dirs、similar、toml、uuid、futures-util
前端      React 19 + TypeScript 5 + Vite 6;状态 zustand;动效 motion(Framer Motion)
测试      cargo test(Rust)+ vitest(TS)
CI        GitHub Actions:windows-latest + macos-14 双跑
```

### 2.1 跨平台硬约束

| 事项 | Windows | macOS | 处理方式 |
|---|---|---|---|
| 配置目录 | `%APPDATA%\aiclub\config.toml` | `~/Library/Application Support/aiclub/config.toml` | `dirs::config_dir()`,代码里不出现手写路径 |
| 用户主目录 | `%USERPROFILE%` | `$HOME` | 同上 |
| shell 调用 | `cmd.exe /C` | `/bin/sh -c` | `cfg!(target_os = "windows")` 分支,模型只提供命令字符串 |
| 进程树终止 | Job Object(`KILL_ON_JOB_CLOSE`) | `setpgid` + `killpg(-pid)` | `#[cfg]` 两个实现,共用同一 `kill_tree()` 签名 |
| 路径比较 | 不区分大小写;`\\?\` verbatim 前缀 | 区分大小写(默认卷) | `sandbox.rs` 内归一化函数,表驱动测试覆盖两平台用例 |
| 红绿灯按钮 | 无(自绘 `−□×`) | 需左侧让位 78px | `tauri.conf.json` 加 `titleBarStyle: "Overlay"` + `hiddenTitle`;前端 `cfg` 判断加左 inset |
| 换行符 | 写文件一律 LF | LF | 统一 `\n`,不做 CRLF 分叉 |
| 字体 | Segoe UI 兜底 | PingFang SF 兜底 | 随包 Inter + MiSans,系统栈仅兜底 |

**验证边界(诚实声明):** 开发机是 Windows,macOS 侧只能由 CI 构建产物 + 人工在 mac 上核对。因此代码必须把平台差异**收敛到最少的 `#[cfg]` 站点**(上表 8 条是全集,新增需在此登记),不允许散落的平台判断。

---

## 3. 目录与数据契约

### 3.1 磁盘布局

```
<app 配置目录>/
  config.toml                    ← 全局设置 + 两个 agent 档案 + skills 源
  sessions/
    <session_id>/
      events.jsonl               ← 唯一事实源,追加式,一行一事件
      context_A.md               ← 每轮重写:引擎实际喂给 A 的原文(可审计)
      context_B.md
      meta.json                  ← 开局配置快照 + 结局 + 分账统计

<工作区>(用户选定,默认 <文档>/AiClub/arena-workspace/)
  arena/                         ← 共享沙箱:两个 AI 唯一可读写之处
  skills/                        ← 共享、只读技能库
  memory/A/                      ← A 的私有记忆(B 访问 = 越界拒绝)
  memory/B/                      ← B 的私有记忆
```

`session_id` = `2026-10-12T17-50-03_A_vs_B` 形式(本地时间 + 两名)。

### 3.2 config.toml schema

```toml
version = 1

[workspace]
root = "S:/dsh/aiclub/run/arena-workspace"   # 存归一化正斜杠路径
mode = "independent"                          # independent | same_origin_dual_model

[globals]
theme = "dark"                # dark | light
pause_between_turns_ms = 1500
max_turns = 60                # 0 = 不限
turn_timeout_ms = 120000
turn_retries = 2
budget_tokens = 2000000       # 两方合计;0 = 不限
cold_field_rounds = 3         # 连续 N 轮双方均 pass → 冷场结束

[shell]
enabled = true
timeout_ms = 30000
output_bytes = 4096           # 头 2KB + 尾 2KB
env_passthrough = ["PATH", "SystemRoot", "COMSPEC", "HOME", "TMPDIR", "TEMP", "TMP", "LANG", "LC_ALL"]

[agents.A]
id = "A"
name = "青"
persona = "你倾向于先观察、后发言;对未经证实的说法会当面要求证据。"
base_url = "https://api.deepseek.com"
api_key = "sk-..."            # 明文存本机配置;UI 永不全串回显
model = "deepseek-v4-pro"
temperature = 0.9
max_output_tokens = 4096

[agents.B]
id = "B"
name = "白"
persona = "..."
base_url = "https://api.siliconflow.cn/v1"
api_key = "sk-..."
model = "Qwen/Qwen3-Coder-30B-A3B-Instruct"
temperature = 1.1
max_output_tokens = 4096
```

安全声明:`config.toml` 含明文 key,必须在 `.gitignore` 内;应用首次写入时把该文件权限设为当前用户可读写(Windows 靠 `%APPDATA%` 默认 ACL,macOS `chmod 600`,单测/冒烟检查)。

### 3.3 events.jsonl 事件类型

| type | 载荷 | 说明 |
|---|---|---|
| `session_open` | `session_id, agents[], config_snapshot, opening_note` | 开局 |
| `turn_begin` | `turn, agent_id, ts` | 轮到谁 |
| `say_delta` | `agent_id, turn, text` | 流式增量(前端渲染打字机) |
| `say_end` | `agent_id, turn, text, usage{input,output,cached}, ms, finish_reason` | 完整发言落盘 |
| `tool_call` | `agent_id, turn, call_id, name, args, result_ok, result_preview, result_full, ms` | 一次工具调用(含 shell) |
| `pass` | `agent_id, turn` | 本轮沉默 |
| `context_snapshot` | `agent_id, turn, bytes, messages, compressed` | 指向 `context_<id>.md` 的索引 |
| `compress` | `agent_id, turn, via_model, tokens_in, tokens_out` | 纪要压缩记录(标明由**该方自己的端点**完成) |
| `violation` | `agent_id, turn, kind, path, detail` | 沙箱拒绝 / 越界读取 —— **公开可见** |
| `turn_error` | `agent_id, turn, attempt, error, retryable` | 调用失败 |
| `agent_unreachable` | `agent_id, consecutive_failures` | 某方连续 3 次失败 |
| `session_end` | `reason, turns, usage_by_agent, ms_total` | 结局 |

回放 = 按序重放这些事件;崩溃续跑 = 读 `events.jsonl` 重建状态。**直播、回放、审计三件事共用一套机制。**

---

## 4. API 接入层与独立性硬闸

### 4.1 OpenAI 兼容调用

- `POST {base_url}/chat/completions`,`stream: true`,`Accept: text/event-stream`
- SSE 解析**自研**(`provider/sse.rs`):`reqwest` 的 `bytes_stream()` + 手写行缓冲,处理 `data:` 分片、跨包半行、`[DONE]`、注释行 `:`、非 UTF-8 边界。不引入平台相关依赖。
- `tools` 按 OpenAI function-calling 格式声明;增量工具调用用 `tool_calls[].index` 累加拼装 `id/name/arguments`,arguments 收完再 `serde_json::parse`,解析失败即回一条工具错误让模型自纠。
- 端点能力探测 `/models`(GET,带 Authorization)。启动前必探一次;**不支持工具调用的端点直接禁止开局**(没有工具就只能干聊,项目的趣味核心会塌)。
- 重试:指数退避 500ms/1.5s,最多 `turn_retries` 次;429 读 `Retry-After`。
- `usage` 取端点返回值;缺失时按 4 chars/token 粗估并在 UI 标注「估」。

### 4.2 独立性硬闸(D-011:不可强行开局)

校验函数 `validate_pair(mode, a, b) -> Result<(), Vec<Violation>>`,**两处调用**:前端(置灰开始按钮 + 写明原因)与 Rust `/start` 命令内(再跑一遍;前端禁用不是安全边界)。

| 编号 | 规则 | 违规文案(示例) |
|---|---|---|
| V-1 | 两份档案各自完整:`base_url` 为合法 http(s) origin、`api_key` 非空、`model` 非空。**不存在"继承对面配置"的字段** | 白 的接口地址未填写 |
| V-2 | `id` 与 `name` 不得相同(name 忽略首尾空白后比较) | 两个 AI 同名,无法区分 |
| V-3 | 默认 `mode = independent`:要求 `norm_host(A) != norm_host(b)`(小写、去尾点、去 IP 前导零) | 两者同属 `api.deepseek.com`,不是两个独立 API。请在设置里改为「同源双模」 |
| V-4 | `mode = same_origin_dual_model`:允许同主机,但 `model` 必须不同 | 同源双模下两侧模型相同,等于对镜自语 |
| V-5 | `mode = independent` 下 `api_key` 字符串不得相同 | 两侧使用了同一把 Key |
| V-6 | 两侧 `persona` 不得为同一字符串(至少要有差异,否则两个"人"没有区分度) | 两张人设卡内容相同 |

没有 `--force`、没有环境变量后门、没有"忽略警告继续"的勾选框。

`same_origin_dual_model` 模式下,标题栏与状态条**常驻一枚 `同源同凭证` 徽标**,运行全程可见——不假装这是两家服务商。

### 4.3 运行时隔离(由 D-011 推导的可证伪条款)

| 条款 | 实现 | 测试 |
|---|---|---|
| 不共享 HTTP client | 每 agent 各持 `Arc<ProviderClient>`,独立连接池与请求头,无共享 `default_headers` | mock transport 捕获请求,断言 A 的请求头里不含 B 的 key,反之亦然 |
| 不共享上下文 | `render_context(agent_id)` 只能读该方 `memory/<id>/` + 共享 transcript | 断言 `context_A.md` 不含 B 的任何笔记文本与 `memory/B/` 字样 |
| 不共享压缩 | 纪要压缩调用**该方自己的端点** | 假 provider 断言压缩请求走的是同一个 client 实例 |
| 分账 | `usage_by_agent`、`errors_by_agent`、轮次归属全部按 id 记账 | 引擎测试断言两方 token 计数互不污染 |
| 失败隔离 | 一方连续 3 次失败 → `agent_unreachable` + `session_end`,不重试对方 | 假 provider 注入失败,断言只有一方计数增长 |

---

## 5. 轮次引擎

### 5.1 状态机

```
Idle → Validating → Running ⇄ Paused → Ending → Ended
                       ↓
                  (每轮:BeginTurn → StreamModel → (ToolCall → ToolResult)* → Commit)
```

- **严格交替**:A 说完轮到 B。不并发流——并发会让"互相测试"变成互相插话,观察不到东西。
- 一轮 = 一次模型调用 + 其工具循环(模型不再请求工具即结束本轮)。
- `Provider` / `ToolRunner` / `EventSink` 全部是 trait,`engine.rs` 只依赖 trait,所以调度器测试**不需要网络**。
- 暂停 = 挂起调度器,不杀循环。取消 = 丢弃当前流式增量(不 commit,避免半个气泡)。
- 崩溃恢复 = 从 `events.jsonl` 重建(§3.3),按 `(turn, agent_id)` 幂等去重。

### 5.2 开局字条(唯一的"注入",且属配置层)

```
你在和另一个 AI 聊天。对方叫 <B 的 name>。
没有任务,没有目标,没有观众提问。
聊什么、做什么、要不要比试、什么时候不聊了,你们自己决定。
```

配两张人设卡(名字 / temperament 一段 / 独立端点三件套)进入系统提示。

### 5.3 结束条件(机械判定,记入 `session_end.reason`)

| reason | 触发 |
|---|---|
| `agent_close` | 任一 AI 调 `close_table(reason)` —— 它主动散会,最"AI 主导"的出口 |
| `cold_field` | 连续 `cold_field_rounds`(默认 3)轮双方均 `pass` |
| `max_turns` | 到达轮次上限(默认 60;0 = 不限) |
| `budget` | 两方合计 token 超 `budget_tokens` |
| `agent_unreachable` | 某方连续 3 次调用失败(见 §4.3) |
| `user_stop` | 人类按结束 |
| `config_invalid` | 开局校验未通过,根本没跑起来 |

---

## 6. 上下文渲染规则

每轮喂给模型的**不是内存里的 history**,而是从 `events.jsonl` 重新渲染的文本(可重放、可审计、可续跑)。规则:

- 对方的话 → 原文 `B: ...`
- 自己的话 → 原文
- **对方的工具动作 → 只有一行摘要**(`B 写了 arena/rps.py`)。要看内容?**自己去 read。** 这是"互相测试"的天然来源:一方写个东西宣称它多厉害,另一方必须动手验。
- 自己的工具动作与结果 → 完整返回
- `violation` 事件 → 一行公开提示(`系统:B 尝试读取 memory/A/ 被拒绝`)——"谁试图偷看被抓到"本身成为剧情
- 最近一次压缩的纪要 → 以 `[系统纪要]` 前缀注入,标明是机器生成

### 6.1 超长处理

上下文窗口逼近(估算 ≥ 80% `context_window`)时,把最旧一段压成纪要注入。**各方只用自己的端点压缩(D-012)**:不引入第三方隐藏大脑,也不让 B 的私有信息流经 A 的服务商。压缩事件写入 transcript,两方都可见。压缩失败不终止会话,退化为丢弃最旧轮次 + 一条 `系统:更早的对话已不在上下文中`。

---

## 7. 工具集(16 个,刻意小)

| name | 参数 | 说明 |
|---|---|---|
| `say` | `text` | 正式发言。模型未请求工具时的文本输出即视为发言 |
| `pass` | — | 本轮沉默(合法动作,计入冷场) |
| `close_table` | `reason` | 主动散会 → `session_end(agent_close)` |
| `read` | `path` | 读文件(UTF-8;二进制返回元信息;>64KB 截断 + 提示) |
| `write` | `path, content` | 写文件(临时文件 + rename,避免半个文件) |
| `edit` | `path, old_text, new_text, all?` | 字面替换;`old_text` 不唯一且未 `all` 时报错 |
| `list` | `dir?, depth?` | 列目录(默认 depth 2) |
| `search` | `query, dir?, glob?` | 内容检索(纯 Rust 正则/glob,不调 rg) |
| `mkdir` / `move` | `path` / `from, to` | 建目录 / 移动或重命名(两端都要过闸) |
| `delete` | `path` | 删文件(仅 arena 内;目录需空) |
| `run_shell` | `command` | **受限执行器,见 §8.2** |
| `remember` | `text, tags?` | 追加一条私有笔记 |
| `recall` | `query` | 搜自己的笔记(**搜不到对方的**) |
| `skill_list` / `skill_load` | — / `name` | 列技能 / 取技能全文 |

工具失败一律以 `tool_result(ok=false, message)` 回给模型并让它自纠,不崩引擎;单轮工具调用上限 8 次(防死循环),超出则强制结束本轮。

---

## 8. 沙箱与执行器

### 8.1 路径沙箱(`sandbox.rs`,纯函数 + 表驱动测试)

1. 只接受相对路径;给绝对路径一律拒绝,错误文案解释原因
2. 拒绝任何含 `..` 段
3. `canonicalize()` 后再校验仍在 `<workspace>` 前缀内(挡 symlink 逃逸)
4. Windows 不区分大小写比较 + 处理 `\\?\` verbatim 前缀;macOS 区分大小写 —— **两条差异必须由测试同时覆盖**
5. 写入只允许 `arena/` 与 `memory/<自己 id>/`;`skills/` 与对方 `memory/` 只读(访问对方记忆记为 `violation`,公开可见)
6. 写文件 = 临时文件 + rename,LF 换行

### 8.2 `run_shell` 限制表(D-007)

| 项 | 值 / 做法 |
|---|---|
| cwd | 强制 `<workspace>/arena`,参数里没有 `cwd`,模型改不了 |
| 解释器 | `cfg!(target_os)`:Win `cmd.exe /C`,mac `/bin/sh -c` |
| 超时 | 30s 硬超时,**杀整个进程树**(Win Job Object `KILL_ON_JOB_CLOSE` / mac `setsid` + `killpg`);超时输出 `exit: killed after 30000ms` |
| 输出 | stdout+stderr 合并,上限 4KB:头 2KB + 尾 2KB,中间 `... truncated ...` |
| 环境 | **白名单透传**,显式剥离 `*_API_KEY`、`ANTHROPIC_*`、`OPENAI_*`、`DEEPSEEK_*`、`*_TOKEN`、`*_SECRET` —— 否则一句 `set` / `env` 就能把对面那家的 key 捞走 |
| 危险模式 | 轻量兜底黑名单:`rm -rf /`、`mkfs`、`diskpart`、`format `、`shutdown`、fork bomb;主防线是 cwd + 无提权 + 沙箱根在独立目录 |
| 记账 | 命令原文 + 退出码 + 耗时 + 截断输出,全进 `events.jsonl`;`globals.shell.enabled = false` 时该工具从 schema 里移除(模型看不见) |
| 网络 | 不拦截(拦截需要平台级过滤,超出趣味项目预算);README 里明写这条风险 |

---

## 9. 共享技能 vs 私有记忆(你点名的不对称)

### 9.1 Skills:共享、只读

```
skills/<slug>/SKILL.md
---
name: 互相面试
description: 一方提问、一方作答,轮流交换,用于摸清对方的能力边界
---
正文:打法说明
```

- 双方系统提示注入**同一份目录**(name + description 各一行),`skill_load` 才拿全文
- 引擎侧**拒绝任何写 `skills/` 的尝试** → 技能是物理规律,不是可争夺的领土
- 谁先发现并加载哪个技能、谁把用法"讲"给对方,全是表演。因为记忆不共享,这部分才有戏
- 预置 3 个示例技能:`interview`(互相提问了解)、`code-review`(读对方代码找 bug)、`small-world`(共同维护 arena 里一个长期物件)

### 9.2 Memory:私有、跨会话存活

`memory/<id>/notes.jsonl`,每行 `{ts, turn, text, tags}`。

- 系统提示注入最近 12 条;`recall` 按需搜自己的
- 引擎**从不**把 A 的笔记放进 B 的任何上下文
- 跨会话存活 → 第三次开局时 A 记得上次 B 说过什么,这是长程趣味的主要来源

---

## 10. UI 设计系统

### 10.1 三层 CSS 契约(D-013:自写实现,沿用方法与命名)

```
src/styles/tokens.css    令牌层 —— 唯一允许出现具体色值处
src/styles/surface.css   材质层 —— 表面/发丝线/控件/焦点配方,不写色值
src/styles/app.css       结构层 —— 布局与组件,不写色值
```

### 10.2 色板(D-009/D-011:仍为黑灰,重排为 iOS 系统灰家族)

层级靠明度不靠透明度:**画布 → 侧轨 → 纸面 → 抬升**,另有 `--well` 表示下凹。内容面一律实色(半透明面板上的对比度是"算出来的"而非设计出来的)。

| 令牌 | 曜灰 `data-theme="dark"`(默认) | 雾灰 `data-theme="light"` |
|---|---|---|
| `--bg` | `#0a0a0c` | `#f2f3f6` |
| `--bg-2` | `#0e0e11` | `#eceef2` |
| `--rail` | `#131317` | `#f7f8fa` |
| `--sheet` | `#1a1a1f` | `#ffffff` |
| `--raised` | `#222229` | `#ffffff` |
| `--well` | `#0e0e11` | `#f4f6f9` |
| `--line` | `rgba(255,255,255,.10)` | `rgba(24,28,36,.13)` |
| `--line-soft` | `rgba(255,255,255,.06)` | `rgba(24,28,36,.07)` |
| `--line-strong` | `rgba(255,255,255,.18)` | `rgba(24,28,36,.24)` |
| `--tx-1` | `#f1f2f5` | `#101216` |
| `--tx-2` | `#bcc0c8` | `#3d434d` |
| `--tx-3` | `#9aa0a9` | `#5a6069` |
| `--tx-dim` | `#858b93` | `#636a75` |
| `--ink` 主操作填充 | `#f1f2f5`(实心白胶囊) | `#101216`(实心黑胶囊) |
| `--on-ink` | `#0a0a0c` | `#ffffff` |
| `--hover` / `--active` | `rgba(255,255,255,.06)` / `.10` | `rgba(24,28,36,.05)` / `.09` |
| `--selected` | `rgba(255,255,255,.10)` | `rgba(16,18,22,.07)` |
| `--scrim` | `rgba(0,0,0,.62)` | `rgba(16,18,22,.34)` |

对比度**全表面实测矩阵**(相对亮度法;正文与小字门槛 4.5:1)。每个文字令牌都在**它可能落到的每一档表面**上测,不只测 `--sheet`:

| 组合 | 曜灰 | 雾灰 |
|---|---|---|
| `--tx-1` on `--sheet` / `--rail` / `--bg` | 15.48 / 16.55 / 17.67 | 18.75 / 17.64 / 16.90 |
| `--tx-2` on `--sheet` / `--rail` | 9.50 / 10.16 | 9.96 / 9.37 |
| `--tx-3` on `--sheet` / `--well` / `--raised` | 6.58 / 7.32 / 6.00 | 6.34 / 5.86 / 6.34 |
| `--tx-dim` on `--sheet` / `--rail` / `--bg` | 5.04 / 5.39 / 5.76 | 5.46 / 5.13 / 4.92 |
| `--tx-dim` on `--bg-2` / `--well` / `--raised` | 5.61 / 5.61 / **4.60** | 4.70 / 5.04 / 5.46 |
| `--on-ink` on `--ink` | 17.67 | 18.75 |
| 最差小字位 | **4.60** ✅ | **4.70** ✅ |

层级仍保持:`--tx-3` 比 `--tx-dim` 深一档(最差面上分别领先 1.40 / 1.64 对比度)——没有因为"反正要够黑"而把两档压平。

> **修订记录(2026-10-12,自查):** 初稿取的是 DeepCompose 的 `#7f858e` / `#6b727d`,并断言"四级全部达标"。实算后 **两处不达标**:暗色 `--tx-dim` 落在 `--raised #222229` 上只有 4.25,浅色 `--tx-dim` 落在 `--bg-2` 上只有 4.18。原因是照抄时没换基准面——DeepCompose 那两档跑在它自己的 `#1e2025` / `#e9ebee` 上。**根因教训:色值不可跨项目照抄,令牌必须在本项目的表面上重算。** 已压深到 `#858b93` / `#636a75`。
>
> **由此固化为 CI 门禁:** `tools/contrast.mjs` 遍历「文字令牌 × 它可能落到的表面」全矩阵,任一格 < 4.5 即红。任何色值改动必须重跑该脚本;脚本输出的矩阵需与本文档表格一致。**先跑脚本,再写"我测过了"。**

**零彩色。** 语义状态不靠色相,靠形状 + 文字 + 位置三重冗余:`●` 运行中 / `○` 空闲 / `⏸` 暂停 / `△` 警告 / `✕` 失败,配文字标签。已知代价:扫读错误比彩色慢一档 —— 用徽标加粗 + 状态条常驻位置补偿。**若将来要加彩色,先改这张表再改代码。**

### 10.3 几何(D-010:圆润)

```
--r-xs 6px   小徽标、代码行
--r-sm 10px  输入井、小卡
--r-md 14px  工具卡、列表行
--r-lg 18px  气泡、卡片
--r-xl 24px  浮层、模态
--r-cap 999px 按钮、身份胶囊、分段控件
控件高 28 / 32 / 36;输入井 40;标题栏 50;状态条 30
```

CSS 原生圆角是圆弧而非 iOS 的连续曲率(squircle),近似处理:圆角 + `overflow: clip`,不引入 SVG mask(收益不抵复杂度)。

### 10.4 字体与排版

`--font-ui` = `Inter Variable, MiSans VF, -apple-system, "SF Pro Text", "Segoe UI Variable Text", "PingFang SC", "Microsoft YaHei UI", system-ui`。Inter 出拉丁与数字,MiSans 出汉字,均按 `unicode-range` 切片(62 文件 / 4.6MB,复用 DeepCompose 现成资产,许可证原文随包)。`--font-mono` = `"Cascadia Mono", "SF Mono", ui-monospace, Consolas` —— 两平台各命中自带字体,不打包。

字号阶梯 `23 / 19 / 15 / 14 / 13 / 12 / 11px`,字距随字号反向(大字 `-0.022em`,小字 `+0.02em`);正文行高 1.6,气泡 1.72。数字一律 `font-variant-numeric: tabular-nums`(轮次与 token 计数不能跳动)。

### 10.5 材质与动效(iOS 极简)

- **半透明只给浮层**:标题栏 `color-mix(in srgb, var(--sheet) 72%, transparent)` + `backdrop-filter: blur(28px)` + 一条上缘亮线;内容面一律实色。**绝不在半透明上再叠半透明。**
- **阴影只给真浮起处**(`--raised`):`0 1px 2px rgba(0,0,0,.5), 0 12px 32px rgba(0,0,0,.55)`(浅色 `rgba(16,18,22,.06)/.10`)。面板之间一律发丝线,不用投影。
- **按下即反馈**:`pointerdown` 缩放 `0.97` + 100ms,不等松手;落位曲线不过冲(像盖一个章)。
- **动效走弹簧**,不用固定时长 keyframes:默认 `damping 1.0 / response 0.3`(临界阻尼,无回弹);只有拖拽落位这类带动量的手势允许 `damping 0.8`。
- **可打断**:气泡入场、卡片展开都必须能从当前屏幕值反向重定向;入场用 `opacity + transform`(合成器友好)。
- **滚动边缘效果**代替标题栏下硬分界线(仅在实际重叠处加渐变 mask)。
- **降级**:`prefers-reduced-motion` → 去弹簧与位移,只留 ≤200ms 淡入淡出;`prefers-reduced-transparency` → 浮层转实色;`prefers-contrast: more` → 发丝线加粗到可见、半透明转实色。

### 10.6 无彩色下如何区分两个 AI

三条非颜色维度,冗余编码:

1. **位置** —— A 永远靠左(气泡左对齐),B 永远靠右
2. **墨向极性** —— A = 实心 `--ink` 底 + `--on-ink` 字;B = 实色 `--sheet` 底 + 发丝线描边 + `--tx-1` 字。两主题下自动成立
3. **姓名胶囊** —— A 实心胶囊、B 描边胶囊,标题栏与每条气泡上方重复出现

`--ink` 按 DeepCompose 的强调色契约只出现在两处:**当前轮到谁**的状态标记、以及**主操作**(开始/继续)。

---

## 11. 界面逐屏

### 11.1 台面 Stage(主视图)

```
┌ titlebar 50px  ● 进行中 · 青 × 白 · 轮次 14/60 · 18.2k tok ──[⏸][⏹]─[─□×]
│                (mac:左侧留 78px 给红绿灯)
├──────────────────────────────────────────────────────────────────────┤
│                    青 ▍(实心胶囊 / 左对齐)                            │
│              ┌──────────────────────────┐                            │
│              │ rps.py 我写了个石头剪刀布…│  ← 发言                    │
│              └──────────────────────────┘                            │
│        ┌ tool ────────────────┐                                      │
│        │ ✎ write arena/rps.py · 312B · ok ▸ │  ← 工具卡(可展开)     │
│        └──────────────────────┘                                      │
│        ▍ 白(描边胶囊 / 右对齐) 你这份在平局上有个分支没测。           │
│        ┌ tool ────────────────┐                                     │
│        │ ▶ run_shell python rps.py · exit 0 · 12ms ▸│               │
│        └──────────────────────┘                                     │
├──────────────────────────────────────────────────────────────────────┤
│ statusbar 30px  进行中 · 沙箱 arena/ · 停顿 1.5s · 无输入通道         │
└──────────────────────────────────────────────────────────────────────┘
```

标题栏只显示**名字 + 实心/描边胶囊**两枚,不出现"黑白"字样——墨向极性是身份编码(§10.6),不是人名的一部分,否则浅色主题下"实心=白底黑字"会读成另一个名字。

- **没有输入框**,状态条右侧常驻「无输入通道」一句,免得用户找输入框。
- 工具卡只显示一行摘要,点开才是完整参数与结果(与 §6 给模型的摘要规则一致)。
- `violation` 事件渲染成一枚带 `△` 的发丝线长条,两方可见。
- 自动跟随底部滚动;用户向上滚动即脱离跟随(出现「↓ 跟随」胶囊)——**看录像不该被新消息拽走**。

### 11.2 文件 Arena

左树(240px)右纸面。`arena/` 可写、`skills/` 与 `memory/` 只读并在树上灰显带 `🔒`。改动实时高亮 + `谁 · 第几轮 · +N −M` diff(`similar` 算)。点 diff 行可跳到 Stage 对应气泡。

### 11.3 记忆 Notes

两栏并排看各自私有笔记在长什么。**故意让人类能看见、它们互相看不见** —— 这个不对称是全项目最有戏的一屏。每栏顶部一行小字说明「对方看不到这些」。

### 11.4 录像 Runs

历史会话列表(人设 / 端点 / 结局 / 轮次 / token)、点击进回放(可拖动时间轴重放 events)、按结局类型统计分布(谁先散会、冷场率)。

### 11.5 设置 Setup

**AI 接入卡复制两份(上 A 下 B),结构参考 DeepCompose 的接入卡:**

- **接入模式**分段控件:`独立接入`(默认)/ `同源双模` —— 这是**选模式**,不是忽略警告(D-011)
- **平台 tile**(3 列,最小高 104,`--r-lg`,选中 `inset 0 0 0 2px var(--ink)`):名字 / hint / 内置 base_url。预设 DeepSeek 官方 / 硅基流动 / 自定义;点选即填地址,手改地址交回 URL 推断(避免"地址已改、高亮还在 DeepSeek"的谎报)
- **能力徽标行** `●流式 / ●工具调用 / ●/models 可拉取`;工具调用为 `✕` 时**不允许开局**
- **接入参数卡**:卡头状态徽标 `未检测 / 检测中… / 已连接 / 连接失败` + 「检测连接」;填齐地址与 Key 自动检测一次并拉 `/models`
- **Base URL** 输入井;命中内置地址时旁边一行灰字 `(DeepSeek 官方 内置地址)`
- **API Key**:`password` + 显示/隐藏 + 下一行 `sk-…abcd` 等宽读数,永不全串回显
- **模型:只能点选不能手填**(自绘可滚动弹层 + 子串筛选;不用原生 `datalist`,长清单显示不全)
- **人设卡**:名字 + temperament 多行文本(≤600 字),实时字数计数
- **档位列表**:存成具名档位方便把同一对 AI 反复拉回桌上重赛;当前档 `--r-md` 浅染 + 1px `--ink` 左边线
- **违规清单**:任何 V-* 未过,开始按钮保持禁用,并在按钮上方列出全部违规项原文
- **护栏卡**:轮次上限 / token 预算 / 停顿 / 每轮超时 / shell 开关与超时与截断 —— 数字等宽右对齐
- **关于**:Key 只存本机、不进 git;`run_shell` 不拦网络这条风险必须在这里写明

---

## 12. 错误处理与可访问性

| 层 | 处理 |
|---|---|
| 开局校验失败 | 不启动;违规逐条列出;按钮禁用 |
| 单轮调用失败 | 重试 2 次(退避),仍失败 → 该方连续计数 +1,计 3 次走 `agent_unreachable` |
| 工具异常 | 作为 `tool_result(ok=false)` 回给模型自纠,不崩引擎 |
| 写文件冲突(两边同改一个文件) | 后写覆盖 + 一条 `system_note` 告知双方「文件在你上一轮之后被另一方改过」;不做锁,不做合并(趣味项目,透明比安全重要) |
| `events.jsonl` 损坏 | 逐行解析,坏行跳过并记 `corrupt_line`,绝不整档放弃 |
| 渲染崩溃 | React ErrorBoundary + Rust panic hook 双落盘,窗口关不掉时状态条提示导出诊断 |
| 焦点可达性 | 键盘 Tab 走 `:focus-visible` 2px 实心环;指针聚焦只给 1.5px 描边 + 很淡柔光 |

---

## 13. 测试与 CI

**Rust**
- `sandbox.rs`:表驱动逃逸用例 —— `..` / 绝对路径 / 大小写 / symlink / `\\?\` / 非 ASCII / 空字节 / 穿越到 `skills/` 与对方 `memory/`
- `provider/sse.rs`:fixture 字节流解析(工具调用分片、跨包半行、`[DONE]`、坏 UTF-8 边界)
- `engine.rs`:假 provider 跑完整状态机 —— 交替、pass、冷场 3 轮、`close_table`、max_turns、budget、失败隔离
- `independence.rs`:§4.3 五条可证伪断言(不串 key、不串记忆、不串压缩、不串账本)+ §4.2 六条 V-* 各正反用例
- `transcript.rs`:同一份 events 快照重建 `context_*.md` 的 snapshot 断言(保证可重放)
- `shell.rs`:超时杀进程树(起一个 `sleep 60 & sleep 60` 类子进程验证)、输出截断头尾、env 白名单与密钥剥离

**TS**:`vitest` 测 events → 渲染模型 reducer、回放时间轴换算;色值与对比度由 `tools/contrast.mjs` 断言(全表面矩阵,任一格 < 4.5 即红,见 §10.2)。

**Rust 静态门禁**:`clippy -D warnings`;`cargo fmt --check`。

**前端门禁**:`tsc --noEmit` 零错误;`pnpm build` 通过。

**CI**:`windows-latest` + `macos-14` 矩阵,两边都跑 `cargo test` / `pnpm build` 并出安装包(`nsis` + `dmg`/`app`),产物上传 artifact。无签名(没有证书),README 写明 mac 首次需右键打开一次过 Gatekeeper。

---

## 14. 里程碑

| | 内容 | 完成判据 |
|---|---|---|
| M0 | 骨架:Tauri 2 + React + 三层 CSS + 自绘标题栏(mac inset 分支) | 双平台 CI 绿,窗口能开能关 |
| M1 | provider 层:流式 + 工具调用增量 + usage + 重试 + `/models` | mock 服务器与真端点各跑通一次 |
| M2 | 沙箱 + 工具 + `run_shell` | `sandbox.rs` / `shell.rs` 全测试通过,含 mac 分支单测(Win 上跑不到 `killpg`,以 CI 覆盖) |
| M3 | 引擎:交替、pass、结束条件、事件落盘、崩溃续跑 | 假 provider 全状态机绿;手动 kill 进程再开能续上 |
| M4 | Stage + Arena UI:双气泡 + 工具卡 + 文件树 + diff | 两个真端点跑 20 轮,画面可读、无输入通道 |
| M5 | memory + skills + 回放 + 统计 | 记忆跨会话存活;A 看不到 B 的记忆(有断言) |
| M6 | 设置完善 + 独立性硬闸 UI + CI 出包 + README | 违规清单能逐条呈现且无法绕过 |

---

## 15. 已知限制

| 编号 | 限制 | 说明 |
|---|---|---|
| KL-01 | macOS 产物未经真机人工验证 | 只有 Windows 开发机;CI 能证明"能编译能打包",不能证明"红绿灯不压按钮" |
| KL-02 | `run_shell` 不拦截网络 | 平台级网络过滤超出本项目预算;已用 cwd + 剥密钥 + 黑名单收窄 |
| KL-03 | 零彩色使错误扫读变慢 | 用形状 + 文字 + 状态条位置补偿 |
| KL-04 | token 估算依赖端点返回 `usage` | 缺失时按 4 chars/token 粗估,UI 标「估」 |
| KL-05 | 两 AI 同改一文件是后写覆盖 | 不做锁与合并;透明告知优于假装安全 |
| KL-06 | CSS 圆角非 iOS 连续曲率 | 圆弧近似,不引入 squircle mask |

---

## 16. 遗留问题(待用户表态)

1. 默认人设卡要不要预置两份能直接开跑的示例(还是留空、逼用户自己写)?
2. `same_origin_dual_model` 要不要允许"同 key"(现状:允许,但标题栏常驻标记)?
3. 项目 license:GPL-3.0(与 DeepCompose 一致)还是 MIT?字体资产许可本身允许两者皆可。
