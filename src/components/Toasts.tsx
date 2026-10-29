import { useApp } from "../store/app";
import { IconX, IconWarn, IconInfo } from "./Icons";

export default function Toasts() {
  const toasts = useApp((s) => s.toasts);
  const drop = useApp((s) => s.dropToast);
  if (toasts.length === 0) return null;
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div className="toast" key={t.id}>
          {t.kind === "error" ? <IconWarn size={14} /> : <IconInfo size={14} />}
          <span className="grow selectable">{t.text}</span>
          <button className="x" onClick={() => drop(t.id)} aria-label="关闭"><IconX size={12} /></button>
        </div>
      ))}
    </div>
  );
}
