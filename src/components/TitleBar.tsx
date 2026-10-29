import { useApp } from "../store/app";
import * as api from "../lib/bridge";
import { IconMinus, IconSquare, IconX, IconSidebar } from "./Icons";

/**
 * 自绘标题栏(decorations:false)。
 * 这里只做三件事:拖动、窗口控制、显示"同源"常驻标记。
 * 没有任何输入控件 —— 那会违反 D-004。
 *
 * ── 拖动为什么不用 -webkit-app-region(2026-10-14 实测) ──
 * 本机 WebView(macOS 26 / WKWebView)根本不认这个属性:
 *   CSS.supports("-webkit-app-region","drag") === false
 *   CSS.supports("app-region","drag")        === false
 *   往元素上写一遍再读回来是空字符串(声明被解析器丢弃)
 *   扫描 document.styleSheets 共 222 条规则,含 app-region 的规则 0 条
 * 也就是说旧写法里那三条 `-webkit-app-region: drag` 是**死声明**,
 * macOS 上等于完全没有拖动能力。
 *
 * 所以拖动只走一条路:Tauri 注入的 drag.js + `data-tauri-drag-region`。
 * 它的判定规则(tauri-2.12.3/src/window/scripts/drag.js):
 *   裸属性 / "true" → 只有**正好点在这个元素本身**才算拖动区
 *   "deep"          → 子树里任何位置都算
 *   "false"         → 显式禁止
 *   可点击元素(button/input/select/textarea/a/label/…)默认阻断拖动
 * 旧代码把裸属性放在一个空的 .drag 上,于是只有标题栏中间那条空白能拖,
 * 图标、"AiClub" 字、版本号、左侧留白全是死区。
 * 改成在 .titlebar 上用 "deep":整条标题栏都能拖,
 * 而按钮因为是可点击元素仍然照常响应点击,不需要逐个加 "false"。
 */
export default function TitleBar() {
  const info = useApp((s) => s.info);
  const st = useApp((s) => s.st);
  const sidebarOpen = useApp((s) => s.sidebarOpen);
  const toggleSidebar = useApp((s) => s.toggleSidebar);
  const config = useApp((s) => s.config);
  const replaying = useApp((s) => s.replaying);
  const toast = useApp((s) => s.toast);

  // 直接读,不经过 useEffect + state:首帧就得是 macos,
  // 否则交通灯安全区(84px)与隐藏窗控会晚一拍才生效。
  const os = info?.os ?? "";

  const sameOrigin = st?.mode === "same_origin_dual_model" || config?.workspace.mode === "same_origin_dual_model";

  const ctl = (a: "minimize" | "toggleMaximize" | "close") => () => {
    void api.windowControl(a).catch((e) => {
      // 以前这里静默吞掉:ACL 没授权时按钮就是"点了没反应",查不出来。
      // 窗口控制是外壳的基本能力,失败必须让人看见。
      toast(`窗口控制失败(${a}):${e instanceof Error ? e.message : String(e)}`, "error");
    });
  };

  return (
    <header className="titlebar" data-os={os} data-tauri-drag-region="deep">
      <button className="btn icon sm ghost" onClick={toggleSidebar} title={sidebarOpen ? "收起侧栏" : "展开侧栏"} aria-label="切换侧栏">
        <IconSidebar size={15} />
      </button>

      <div className="brand">
        <img src="/appicon.png" alt="" />
        <b>AiClub</b>
        <span className="ver">v{info?.version ?? "0.1.0"}</span>
      </div>

      {replaying && (
        <span className="banner-sameorigin" title="正在回放历史记录,直播已暂停显示">
          回放中 · {replaying}
        </span>
      )}

      {sameOrigin && (
        <span className="banner-sameorigin" title="两侧使用同一服务商。这是你主动选择的模式,不是警告被忽略。">
          同源双模
        </span>
      )}

      {/* 只作为把窗控推到右边的弹性占位;拖动由外层 header 的 deep 区域负责 */}
      <div className="drag" />

      <div className="win-btns">
        <button className="win-btn" onClick={ctl("minimize")} aria-label="最小化" title="最小化"><IconMinus size={15} /></button>
        <button className="win-btn" onClick={ctl("toggleMaximize")} aria-label="最大化" title="最大化"><IconSquare size={13} /></button>
        <button className="win-btn close" onClick={ctl("close")} aria-label="关闭" title="关闭"><IconX size={15} /></button>
      </div>
    </header>
  );
}
