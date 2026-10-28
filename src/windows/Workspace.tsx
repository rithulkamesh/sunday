import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Knowledge, importNotes } from "../views/Knowledge";
import { Memory } from "../views/Memory";
import { Palette, type Cmd, type View } from "../views/Palette";
import { Settings } from "../views/Settings";
import { Skills } from "../views/Skills";
import { Label } from "../ui";

type Msg = { role: "user" | "assistant"; text: string; sources?: string[] };
type State = {
  due: { concept: string; overdue_days: number }[];
  total: number;
  correct: number;
  weak: { concept: string; wrong: number }[];
  recent: { concept: string; verdict: string; ts: number }[];
  pending: string | null;
  coverage: [number, number];
};

const VERDICT = { correct: "text-emerald-400", partial: "text-amber-400", wrong: "text-rose-400" } as Record<string, string>;

const flag = (k: string, d: boolean) => {
  try {
    return (localStorage.getItem(k) ?? String(d)) === "true";
  } catch {
    return d;
  }
};

/** Main window: what you're learning on the left, the conversation (typed or spoken) on the right. */
export function Workspace() {
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [state, setState] = useState<State | null>(null);
  const [busy, setBusy] = useState(false);
  const [text, setText] = useState("");
  const [voice, setVoice] = useState(() => flag("voice", true));
  const [view, setView] = useState<View>("home");
  const [palette, setPalette] = useState(false);
  const end = useRef<HTMLDivElement>(null);

  const refresh = useCallback(() => void invoke<State>("learn_state").then(setState), []);

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

  const ask = (t: string, spoken = voice) => {
    if (!t.trim()) return;
    setView("home");
    setBusy(true);
    void invoke("ask", { text: t, voice: spoken });
  };

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPalette((p) => !p);
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, []);

  const commands: Cmd[] = [
    { title: "Resume last session", hint: state?.pending ? "question waiting" : "", run: () => setView("home") },
    { title: "Teach me…", arg: "What do you want to learn?", run: (t) => t && ask(`teach me ${t}`) },
    { title: "Start a practice question", run: () => ask("quiz me") },
    { title: "Review due concepts", hint: `${state?.due.length ?? 0} due`, run: () => ask("quiz me") },
    { title: "Explain a concept…", arg: "Which concept?", run: (c) => c && ask(`explain ${c}`) },
    { title: "Quiz me on…", arg: "Which topic?", run: (c) => c && ask(`quiz me on ${c}`) },
    { title: "Search local notes…", arg: "Search for", run: (c) => c && ask(`search my notes for ${c}`, false) },
    { title: "Explain what's on my screen", hint: "reads and draws", run: () => ask("explain what's on my screen", true) },
    { title: "Clear drawing on screen", run: () => ask("clear the screen", false) },
    { title: "Remember…", arg: "Remember that…", run: (t) => t && ask(`remember that ${t}`, false) },
    { title: "Read a web page…", arg: "Web address", hint: "web access", run: (u) => u && ask(`read ${u}`, false) },
    { title: "Research…", arg: "What should I look up?", hint: "web access", run: (q) => q && ask(`research ${q}`) },
    { title: "Go to Memory", run: () => setView("memory") },
    { title: "Import notes", run: () => { setView("knowledge"); void importNotes(); } },
    { title: "Go to Home", run: () => setView("home") },
    { title: "Go to Knowledge", run: () => setView("knowledge") },
    { title: "Go to Skills", run: () => setView("skills") },
    { title: "Learn my voice", hint: "only answer to me", run: () => { setView("settings"); void invoke("enroll_voice"); } },
    { title: "Forget my voice", run: () => void invoke("forget_voice") },
    { title: "Open diagnostics", run: () => setView("settings") },
  ];

  const toggleVoice = (v: boolean) => {
    setVoice(v);
    try {
      localStorage.setItem("voice", String(v));
    } catch {}
  };

  const acc = state?.total ? Math.round((100 * state.correct) / state.total) : null;
  const NAV: [View, string][] = [["home", "Home"], ["knowledge", "Knowledge"], ["memory", "Memory"], ["skills", "Skills"], ["settings", "Settings"]];

  return (
    <div className="flex h-full bg-canvas">
      <aside className="flex w-72 shrink-0 flex-col gap-6 overflow-auto border-r border-line bg-panel p-5">
        <div>
          <h1 className="text-lg font-semibold">Sunday</h1>
          <p className="mt-1 text-sm text-dim">Hold fn or say “sunday” · ⌘K</p>
        </div>

        <nav className="flex flex-col gap-1">
          {NAV.map(([v, name]) => (
            <button
              key={v}
              onClick={() => setView(v)}
              className={`rounded-lg px-3 py-1.5 text-left text-sm ${view === v ? "bg-raised" : "text-dim hover:text-ink"}`}
            >
              {name}
            </button>
          ))}
        </nav>

        <section>
          <Label>Learning</Label>
          <div className="grid grid-cols-2 gap-2">
            <Stat n={state?.due.length ?? 0} label="due for review" />
            <Stat n={acc === null ? "–" : `${acc}%`} label={`accuracy · ${state?.total ?? 0} answered`} />
          </div>
          <p className="mt-2 text-xs text-dim">{state ? `${state.coverage[0]} of ${state.coverage[1]} concepts known` : ""}</p>
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
        {view === "knowledge" && <Knowledge onAsk={(t) => ask(t)} />}
        {view === "memory" && <Memory onAsk={(t) => ask(t)} />}
        {view === "skills" && <Skills />}
        {view === "settings" && <Settings />}
        {view === "home" && (
          <>
            <div className="flex-1 select-text space-y-3 overflow-auto p-8">
              {!msgs.length && (
                <p className="text-dim">
                  Ask me anything. Say “teach me …” to learn something new, “explain what's on my screen” and I'll look and point, “research …” to look something up. Press ⌘K for everything else.
                </p>
              )}
              {msgs.map((m, i) => (
                <div key={i} className={m.role === "user" ? "flex justify-end" : "flex"}>
                  <div className={`max-w-[640px] rounded-2xl px-4 py-2.5 leading-snug ${m.role === "user" ? "bg-raised" : "border border-line"}`}>
                    <p className="whitespace-pre-wrap">{m.text}</p>
                    {!!m.sources?.length && <p className="mt-1.5 text-xs text-dim">from {[...new Set(m.sources)].join(" · ")}</p>}
                  </div>
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
                placeholder={state?.pending ? "Type your answer…" : "Ask anything…"}
                value={text}
                onChange={(e) => setText(e.target.value)}
                autoFocus
              />
              <label className="flex items-center gap-2 text-sm text-dim">
                <input type="checkbox" checked={voice} onChange={(e) => toggleVoice(e.target.checked)} />
                speak
              </label>
            </form>
          </>
        )}
      </main>
      {palette && <Palette commands={commands} onClose={() => setPalette(false)} />}
    </div>
  );
}

const Stat = ({ n, label }: { n: number | string; label: string }) => (
  <div className="rounded-lg border border-line bg-raised p-3">
    <div className="text-2xl font-semibold">{n}</div>
    <div className="text-xs text-dim">{label}</div>
  </div>
);

const Btn = (p: React.ButtonHTMLAttributes<HTMLButtonElement>) => (
  <button {...p} className="rounded-lg border border-line bg-raised px-3 py-2 text-sm hover:border-accent" />
);
