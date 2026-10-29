import { useMemo } from "react";
import { useApp } from "../store/app";
import { humanBytes, ZONE_LABEL, looksBinary } from "../lib/format";
import { IconFile, IconFolder, IconBack, IconChevronR } from "../components/Icons";

/**
 * 文件面板。两个 AI 的共同世界都在这里 —— 人类旁观它们的产出。
 * 只读:人不能改 arena/,否则就变成参与者了。
 */
export default function Files() {
  const entries = useApp((s) => s.treeEntries);
  const current = useApp((s) => s.currentFile);
  const path = useApp((s) => s.currentPath);
  const openFile = useApp((s) => s.openFile);
  const closeFile = useApp((s) => s.closeFile);
  const loadTree = useApp((s) => s.loadTree);
  const st = useApp((s) => s.st);

  // 按目录分组渲染,不画真正的树控件 —— 保持简单可读
  const grouped = useMemo(() => {
    const m = new Map<string, typeof entries>();
    for (const e of entries) {
      if (e.dir) continue;
      const slash = e.path.lastIndexOf("/");
      const dir = slash < 0 ? "" : e.path.slice(0, slash);
      if (!m.has(dir)) m.set(dir, []);
      m.get(dir)!.push(e);
    }
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [entries]);

  if (current && path) {
    return (
      <div className="pane">
        <div className="pane-head">
          <button className="btn icon sm ghost" onClick={closeFile} title="返回列表"><IconBack size={15} /></button>
          <span className="title mono ellipsis grow">{path}</span>
          <span className="xxs dim tnum">{humanBytes(current.bytes)}</span>
          {current.truncated && <span className="chip">已截断</span>}
        </div>
        <div className="pane-body">
          <div className="fileview">
            {current.binary ? (
              <div className="notice">
                这是二进制文件({humanBytes(current.bytes)}),不预览内容。
                <br />
                <span className="dim">两个 AI 可以读它,但这里只给人类看文本。</span>
              </div>
            ) : (
              <pre className="code selectable">{current.text || "(空文件)"}</pre>
            )}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="pane">
      <div className="pane-head">
        <IconFolder size={15} />
        <span className="title">共享世界</span>
        <span className="chip">只读</span>
        <span className="grow" />
        <button className="btn sm ghost" onClick={() => void loadTree()}>刷新</button>
      </div>
      <div className="pane-body">
        {entries.filter((e) => !e.dir).length === 0 ? (
          <div className="empty">
            <IconFolder size={26} />
            <div className="big">还是空的</div>
            <div className="xs" style={{ maxWidth: 320, lineHeight: 1.7 }}>
              arena/ 是两个 AI 共同的沙盒。等它们开始写东西,文件会出现在这里。
            </div>
            {st?.workspace && <div className="xxs" style={{ marginTop: 8 }}>{st.workspace}</div>}
          </div>
        ) : (
          <div className="tree">
            {grouped.map(([dir, files]) => (
              <div key={dir || "/"}>
                <div className="tree-row" style={{ color: "var(--tx-dim)", height: 22 }}>
                  <IconChevronR size={12} />
                  <span className="nm">{dir || "工作区根"}</span>
                </div>
                {files.map((f) => (
                  <button
                    key={f.path}
                    className="tree-row"
                    style={{ paddingLeft: 22 }}
                    onClick={() => void openFile(f.path)}
                    title={f.path}
                  >
                    <IconFile size={13} />
                    <span className="nm">{f.path.slice(dir ? dir.length + 1 : 0)}</span>
                    {f.zone && f.zone !== "arena" && <span className="zone">{ZONE_LABEL[f.zone] ?? f.zone}</span>}
                    {/* 二进制文件点开只会得到"已跳过内容读取",所以列表里就标出来,
                        省得用户白点一次。TreeEntry 不带 binary 字段(只有
                        path/dir/bytes/zone),这里按扩展名猜 —— 列表视图不必读文件内容。
                        注:以前这里两个分支都是 humanBytes(f.bytes),三元是死的。 */}
                    <span className="size tnum">
                      {looksBinary(f.path) ? "二进制" : humanBytes(f.bytes)}
                    </span>
                  </button>
                ))}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
