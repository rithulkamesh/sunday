import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export type View = "home" | "knowledge" | "memory" | "skills" | "settings";
/** `arg` is the prompt for a value to ask for first; `run` then receives it. */
export type Cmd = { title: string; hint?: string; run: (arg?: string) => void; arg?: string };

/** ⌘K: every action in one keyboard-driven list. Commands that need a value ask for it next. */
export function Palette({ commands, onClose }: { commands: Cmd[]; onClose: () => void }) {
  const [q, setQ] = useState("");
  const [sel, setSel] = useState(0);
  const [pending, setPending] = useState<Cmd | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    input.current?.focus();
    void invoke<string[]>("models").then(setModels);
  }, []);

  const all = useMemo<Cmd[]>(
    () => [
      ...commands,
      ...models.map((m) => ({ title: `Switch model: ${m}`, run: () => void invoke("set_model", { name: m }) })),
    ],
    [commands, models],
  );
  const shown = pending ? [] : all.filter((c) => c.title.toLowerCase().includes(q.toLowerCase()));

  const choose = (c?: Cmd) => {
    if (!c) return;
    if (c.arg) {
      setPending(c);
      setQ("");
    } else {
      onClose();
      c.run();
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 pt-[14vh]" onMouseDown={onClose}>
      <div className="w-[560px] overflow-hidden rounded-2xl border border-line bg-panel shadow-2xl" onMouseDown={(e) => e.stopPropagation()}>
        <input
          ref={input}
          className="w-full select-text border-b border-line bg-transparent px-5 py-4 text-base outline-none"
          placeholder={pending ? pending.arg : "Type a command…"}
          value={q}
          onChange={(e) => {
            setQ(e.target.value);
            setSel(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape") return onClose();
            if (e.key === "ArrowDown") return setSel((s) => Math.min(s + 1, shown.length - 1));
            if (e.key === "ArrowUp") return setSel((s) => Math.max(s - 1, 0));
            if (e.key !== "Enter") return;
            if (pending) {
              if (!q.trim()) return;
              onClose();
              pending.run(q.trim());
            } else choose(shown[sel]);
          }}
        />
        <ul className="max-h-80 overflow-auto py-1">
          {shown.map((c, i) => (
            <li
              key={c.title}
              className={`flex cursor-pointer justify-between px-5 py-2.5 text-sm ${i === sel ? "bg-raised" : ""}`}
              onMouseEnter={() => setSel(i)}
              onClick={() => choose(c)}
            >
              <span>{c.title}</span>
              <span className="text-dim">{c.hint}</span>
            </li>
          ))}
          {!shown.length && !pending && <li className="px-5 py-3 text-sm text-dim">No matching command</li>}
          {pending && <li className="px-5 py-3 text-sm text-dim">Press Enter to run “{pending.title}”</li>}
        </ul>
      </div>
    </div>
  );
}
