# AiClub macOS 迁移说明

## 目标

AiClub 的 Rust 引擎、Provider、沙箱、事件模型、记忆与技能逻辑保持不变。本适配只增加 macOS 窗口配置、原生标题栏/交通灯避让、系统字体与无障碍透明度降级。

## 构建

在 macOS 上安装 Node.js、pnpm、Rust 与 Xcode Command Line Tools 后：

```bash
pnpm install
pnpm run build
pnpm tauri build --config src-tauri/tauri.macos.conf.json
```

产物为 `.app` / `.dmg`。macOS 配置使用本机 ad hoc 签名（`signingIdentity: "-"`），用于封印 bundle 资源；可用 `codesign --verify --deep --strict` 校验。它不等于 Apple Developer ID 签名或公证，首次打开仍可能需要在 Finder 中右键「打开」，或按系统提示允许。

## macOS UI 适配

- 原生 Overlay 标题栏与交通灯由 `src-tauri/tauri.macos.conf.json` 启用；
- Windows 自绘最小化/最大化/关闭按钮在 macOS 上由 `[data-os="macos"]` 隐藏；
- 标题栏为 macOS 原生半透明材质，`prefers-reduced-transparency` 时退化为实色；
- 字体优先使用 SF Pro / PingFang SC / SF Mono；
- Rust 引擎与 IPC 契约不因平台分支改变；
- macOS 的原生窗口权限由 `src-tauri/capabilities/default.json` 提供。

## 必须在真机确认

- Retina 下标题栏交通灯与品牌标识不重叠；
- 交通灯、窗口最大化、关闭按钮正常；
- TCC 对 Documents / Desktop 工作区访问的授权弹窗；
- 两个真实独立 API 端点的长时对话、暂停、回放与导出。
