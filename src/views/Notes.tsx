import { useApp } from "../store/app";
import { IconNote, IconLock } from "../components/Icons";
import { readableStamp, splitSession } from "../lib/format";

/**
 * 记忆面板。人类可以并排看两份私有记忆 —— 但两个 AI 互相看不见。
 * 这是这个项目最有意思的一屏:你能看到它们各自记得什么、忘了什么。
 */
export default function Notes() {
  const notes = useApp((s) => s.notes);
  const loadNotes = useApp((s) => s.loadNotes);

  return (
    <div className="pane">
      <div className="pane-head">
        <IconNote size={15} />
        <span className="title">私有记忆</span>
        <span className="chip">互相不可见</span>
        <span className="grow" />
        <button className="btn sm ghost" onClick={() => void loadNotes()}>刷新</button>
      </div>

      <div className="pane-body">
        {notes.length === 0 ? (
          <div className="empty">
            <IconNote size={26} />
            <div className="big">还没有记忆</div>
            <div className="xs" style={{ maxWidth: 340, lineHeight: 1.7 }}>
              两个 AI 可以用 remember 把「下次要记得的事」写进自己的记忆。
              记忆跨会话存活,是它们认出彼此的唯一依据。
            </div>
          </div>
        ) : (
          <div className="mem-grid">
            {notes.map((m, i) => (
              <div className="mem-col" key={m.agent}>
                <div className="mem-col-head">
                  <span className={"sigil sm " + (i === 0 ? "a" : "b")}>{m.name.slice(0, 1)}</span>
                  <span>{m.name}</span>
                  <span className="grow" />
                  <span className="row gap-4 dim"><IconLock size={11} /> {m.notes.length} 条</span>
                </div>
                <div className="scroll">
                  {m.notes.length === 0 ? (
                    <div className="xs dim" style={{ padding: 14 }}>这一方还没有写下任何东西。</div>
                  ) : (
                    [...m.notes].reverse().map((n, k) => {
                      const s = splitSession(n.session);
                      return (
                        <div className="note" key={k}>
                          <div className="meta">
                            {readableStamp(s.stamp)} · 第 {n.turn} 轮
                          </div>
                          <div className="text selectable">{n.text}</div>
                          {n.tags.length > 0 && (
                            <div className="tags">
                              {n.tags.map((t) => <span className="chip" key={t}>#{t}</span>)}
                            </div>
                          )}
                        </div>
                      );
                    })
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
