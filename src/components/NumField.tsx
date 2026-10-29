import { useEffect, useRef, useState } from "react";

/**
 * 数字输入框。
 *
 * ── 为什么单独一个文件 ──
 * 这段逻辑以前内联在 Setup.tsx 里,是"模型探测后可选择"那条链路上第三个坑:
 * 旧写法每次按键都 `onChange(max(min, round(n)))`,并且直接把钳制后的值塞回
 * 受控 input,于是"逐字键入"根本不可能 ——
 *   想输 60000,第一击是 "6" → 立刻被钳成 3000(单轮超时 min=3000),
 *   光标停在后面,接着敲出 "300060000"。
 * 实测(2026-10-14,真机 WebView)复现的就是这个数。抽出来是为了能直接写回归测试。
 *
 * 现在的规矩:
 *   · 输入过程中只把**解析后的合法值**提交给草稿(保证配置任何时刻都有效),
 *     但**显示**用本地文本、不做钳制,让用户把数字打完;
 *   · 失焦时再统一收敛:显示的一定是真正生效的值;
 *   · 清空 / 非法输入不提交(不把配置写成 0),失焦时回到当前生效值。
 */
export default function NumField(props: {
  label: string; value: number; step: number; min: number; hint: string;
  onChange: (v: number) => void;
}) {
  const [text, setText] = useState(String(props.value));
  const [editing, setEditing] = useState(false);
  /** 记住"我们自己刚提交过的值",用来区分外部变更和自身回写 */
  const lastCommitted = useRef<number | null>(null);

  useEffect(() => {
    if (editing) return;
    if (lastCommitted.current === props.value) return;
    setText(String(props.value));
  }, [props.value, editing]);

  const commit = (raw: string) => {
    const n = Number(raw);
    if (raw.trim() === "" || !Number.isFinite(n)) return; // 中间态:不提交,也不报错
    const clamped = Math.max(props.min, Math.round(n));
    lastCommitted.current = clamped;
    props.onChange(clamped);
  };

  return (
    <>
      {props.label && <label>{props.label}</label>}
      <div className="num-field">
        <input
          className="input tnum"
          type="number"
          value={text}
          min={props.min}
          step={props.step}
          onFocus={() => setEditing(true)}
          onChange={(e) => { setText(e.target.value); commit(e.target.value); }}
          onBlur={() => {
            setEditing(false);
            const raw = text.trim();
            const n = Number(raw);
            if (raw === "" || !Number.isFinite(n)) {
              setText(String(props.value)); // 清空或非法 → 回到当前生效值
              return;
            }
            const clamped = Math.max(props.min, Math.round(n));
            lastCommitted.current = clamped;
            props.onChange(clamped);
            setText(String(clamped)); // 失焦后显示的必须是真正生效的值
          }}
        />
      </div>
      {props.hint && <span className="hint">{props.hint}</span>}
    </>
  );
}
