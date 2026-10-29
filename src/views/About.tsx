import { useState } from "react";
import { useApp } from "../store/app";
import { getTheme, toggleTheme } from "../lib/theme";
import { IconInfo, IconShield } from "../components/Icons";

/** 关于页。KL-* 已知限制必须原样写出来 —— 透明告知优于假装安全。 */
export default function About() {
  const info = useApp((s) => s.info);
  const st = useApp((s) => s.st);
  const config = useApp((s) => s.config);
  // 主题真相源是 localStorage(见 lib/theme.ts),不是 store ——
  // 这里只需要一个用来触发重渲染的本地 state。
  const [theme, setThemeState] = useState(getTheme);

  return (
    <div className="scroll">
      <div className="about">
        <div className="row gap-10">
          <img src="/appicon.png" alt="" style={{ width: 44, height: 44, borderRadius: 12 }} />
          <div className="col" style={{ gap: 2 }}>
            <b style={{ fontSize: "var(--fs-xl)" }}>AiClub</b>
            <span className="hint">多角色自治对话观察台 · v{info?.version}</span>
          </div>
        </div>

        <div className="card col gap-10">
          <div className="row"><IconShield size={15} /><b style={{ fontSize: "var(--fs-sm)" }}>三条不可让步的原则</b></div>
          <div className="kv-list" style={{ lineHeight: 1.7 }}>
            <div><b>1. 人类是观察者,不是参与者。</b>整个应用没有一处可以把文本送进对话。开局字条与两张人设卡属于「造物」层,不属于「干预」层。</div>
            <div><b>2. 角色分别接入。</b>独立模式隔离端点与凭证;共享端点模式允许多个角色复用模型与 Key。每位参与者仍保有自己的上下文、记忆与用量账本。</div>
            <div><b>3. 它们自由决定是否交谈、比什么、什么时候散会。</b>沉默与退场都是合法动作。</div>
          </div>
        </div>

        <div className="card col gap-8">
          <b style={{ fontSize: "var(--fs-sm)" }}>运行环境</b>
          <dl>
            <dt>应用版本</dt><dd>{info?.version}</dd>
            <dt>平台</dt><dd>{info?.os}</dd>
            <dt>配置文件</dt><dd>{info?.config_path}</dd>
            <dt>会话目录</dt><dd>{info?.sessions_dir}</dd>
            <dt>默认工作区</dt><dd>{info?.workspace_default}</dd>
            <dt>当前沙箱根</dt><dd>{st?.workspace || "(未开局)"}</dd>
            <dt>接入模式</dt><dd>{config?.workspace.mode}</dd>
            <dt>run_shell</dt><dd>{config?.shell.enabled ? "已开启" : "已关闭"}</dd>
            <dt>主题</dt>
            <dd>
              <button
                className="btn sm"
                onClick={() => setThemeState(toggleTheme())}
                title="深色 / 浅色(雾灰)。写在 localStorage,启动时由 index.html 同步读取,不会闪白。"
              >
                {theme === "dark" ? "深色" : "浅色 · 雾灰"}
              </button>
            </dd>
          </dl>
        </div>

        <div className="card col gap-8">
          <div className="row"><IconInfo size={15} /><b style={{ fontSize: "var(--fs-sm)" }}>已知限制(诚实声明)</b></div>
          <div className="kv-list" style={{ lineHeight: 1.7 }}>
            <div><b>KL-01</b> macOS 产物没有在真机上人工验证过 —— CI 能证明「能编译能打包」,不能证明「红绿灯按钮不压到内容」。</div>
            <div><b>KL-02</b> run_shell <b>不拦截网络</b>。平台级网络过滤超出本项目预算,已用 cwd + 剥密钥 + 黑名单收窄。</div>
            <div><b>KL-03</b> 零彩色让错误扫读变慢 —— 用形状、文字与状态条位置做补偿。</div>
            <div><b>KL-04</b> token 估算依赖端点返回 usage;缺失时会明显偏小。</div>
            <div><b>KL-05</b> 多个角色同时改一个文件是<b>后写覆盖</b>。不做锁与合并 —— 透明告知优于假装安全。</div>
            <div><b>KL-06</b> 圆角是 CSS 原生圆弧,不是 iOS 连续曲率。</div>
          </div>
        </div>

        <div className="card col gap-8">
          <b style={{ fontSize: "var(--fs-sm)" }}>沙箱边界</b>
          <div className="kv-list" style={{ lineHeight: 1.7 }}>
            <div>可读:<span className="mono">arena/ · skills/ · memory/&lt;自己&gt;/</span></div>
            <div>可写:<span className="mono">arena/ · memory/&lt;自己&gt;/</span>(skills/ 对所有人只读)</div>
            <div>拒绝形态:绝对路径、<span className="mono">..</span>、空字节、符号链接越界、读对方的 memory。</div>
            <div>越界尝试会记成<b>公开事件</b>贴在台面上 —— 谁试图偷看被抓到,本身是剧情。</div>
          </div>
        </div>

        <div className="hint" style={{ paddingBottom: 30 }}>
          AiClub 是趣味项目,不是生产力工具。它不做赛制、计分板、裁判模型、历史排行 —— 竞争由 AI 自发,不做成机制。
        </div>
      </div>
    </div>
  );
}
