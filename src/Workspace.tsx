import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Msg = { role: "user" | "assistant"; text: string };
type State = {
  due: { concept: string; overdue_days: number }[];
  total: number;
  correct: number;
  weak: { concept: string; wrong: number }[];
  recent: { concept: string; verdict: string; ts: number }[];
  pending: string | null;
};

const VERDICT = { correct: "text-emerald-400", partial: "text-amber-400", wrong: "text-rose-400" } as Record<string, string>;

const flag = (k: string, d: boolean) => {
  try {
    return (localStorage.getItem(k) ?? String(d)) === "true";
  } catch {
    return d;
  }
};

/** Main window: today's study status on the left, the conversation (typed or spoken) on the right. */
export function Workspace() {
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [state, setState] = useState<State | null>(null);
  const [busy, setBusy] = useState(false);
  const [text, setText] = useState("");
  const [voice, setVoice] = useState(() => flag("voice", true));
  const end = useRef<HTMLDivElement>(null);

  const refresh = useCallback(() => void invoke<State>("study_state").then(setState), []);

  useEffect(() => {
    void invoke<Msg[]>("history", { limit: 50 }).then(setMsgs);
    refresh();
    const sub = listen<Msg>("turn", (e) => {
      setMsgs((m) => [...m, e.payload]);
      setBusy(e.payload.role === "user");
      refresh();
    });
    return () => void sub.then((off) => off());
  }, [refresh]);

  useEffect(() => end.current?.scrollIntoView({ behavior: "smooth" }), [msgs, busy]);

  const ask = (t: string) => {
    if (!t.trim()) return;
    setBusy(true);
    void invoke("ask", { text: t, voice });
  };

  const toggleVoice = (v: boolean) => {
    setVoice(v);
    try {
      localStorage.setItem("voice", String(v));
    } catch {}
  };

  const acc = state?.total ? Math.round((100 * state.correct) / state.total) : null;

  return (
    <div className="flex h-full bg-canvas">
      <aside className="flex w-72 shrink-0 flex-col gap-6 overflow-auto border-r border-line bg-panel p-5">
        <div>
          <h1 className="text-lg font-semibold">Sunday</h1>
          <p className="mt-1 text-sm text-dim">Hold fn or say “sunday”</p>
        </div>

        <section>
          <Label>Today</Label>
          <div className="grid grid-cols-2 gap-2">
            <Stat n={state?.due.length ?? 0} label="due for review" />
            <Stat n={acc === null ? "–" : `${acc}%`} label={`accuracy · ${state?.total ?? 0} answered`} />
          </div>
          <div className="mt-3 flex gap-2">
            <Btn onClick={() => ask("quiz me")}>{state?.pending ? "New question" : state?.due.length ? "Review due" : "Quiz me"}</Btn>
          </div>
          {state?.pending && <p className="mt-3 text-sm text-dim">Waiting for your answer: {state.pending}</p>}
        </section>

        {!!state?.due.length && (
          <section>
            <Label>Due</Label>
            <ul className="space-y-1 text-sm">
              {state.due.slice(0, 6).map((d) => (
                <li key={d.concept} className="flex justify-between">
                  <span>{d.concept}</span>
                  <span className="text-dim">{d.overdue_days > 0 ? `${d.overdue_days}d late` : "today"}</span>
                </li>
              ))}
            </ul>
          </section>
        )}

        {!!state?.weak.length && (
          <section>
            <Label>Weak spots</Label>
            <ul className="space-y-1 text-sm">
              {state.weak.map((w) => (
                <li key={w.concept} className="flex justify-between">
                  <span>{w.concept}</span>
                  <span className="text-dim">{w.wrong} missed</span>
                </li>
              ))}
            </ul>
          </section>
        )}

        {!!state?.recent.length && (
          <section>
            <Label>Recent</Label>
            <ul className="space-y-1 text-sm">
              {state.recent.map((r, i) => (
                <li key={i} className="flex justify-between">
                  <span>{r.concept}</span>
                  <span className={VERDICT[r.verdict]}>{r.verdict}</span>
                </li>
              ))}
            </ul>
          </section>
        )}
      </aside>

      <main className="flex min-w-0 flex-1 flex-col">
        <div className="flex-1 select-text space-y-3 overflow-auto p-8">
          {!msgs.length && (
            <p className="text-dim">
              Ask about a concept, or say “quiz me on graphs”. Answers, mistakes and review dates are saved.
            </p>
          )}
          {msgs.map((m, i) => (
            <div key={i} className={m.role === "user" ? "flex justify-end" : "flex"}>
              <p
                className={`max-w-[640px] whitespace-pre-wrap rounded-2xl px-4 py-2.5 leading-snug ${
                  m.role === "user" ? "bg-raised" : "border border-line"
                }`}
              >
                {m.text}
              </p>
            </div>
          ))}
          {busy && <p className="text-sm text-dim">thinking…</p>}
          <div ref={end} />
        </div>
        <form
          className="flex items-center gap-3 border-t border-line p-4"
          onSubmit={(e) => {
            e.preventDefault();
            ask(text);
            setText("");
          }}
        >
          <input
            className="flex-1 select-text rounded-xl border border-line bg-raised px-4 py-2.5 outline-none focus:border-accent"
            placeholder={state?.pending ? "Type your answer…" : "Ask, or type “quiz me”…"}
            value={text}
            onChange={(e) => setText(e.target.value)}
            autoFocus
          />
          <label className="flex items-center gap-2 text-sm text-dim">
            <input type="checkbox" checked={voice} onChange={(e) => toggleVoice(e.target.checked)} />
            speak
          </label>
        </form>
      </main>
    </div>
  );
}

const Label = ({ children }: { children: string }) => (
  <h2 className="mb-2 font-mono text-xs uppercase tracking-widest text-dim">{children}</h2>
);

const Stat = ({ n, label }: { n: number | string; label: string }) => (
  <div className="rounded-lg border border-line bg-raised p-3">
    <div className="text-2xl font-semibold">{n}</div>
    <div className="text-xs text-dim">{label}</div>
  </div>
);

const Btn = (p: React.ButtonHTMLAttributes<HTMLButtonElement>) => (
  <button {...p} className="rounded-lg border border-line bg-raised px-3 py-2 text-sm hover:border-accent" />
);
