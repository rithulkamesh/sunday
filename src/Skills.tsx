import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Label } from "./ui";

type Skill = {
  id: string;
  version: number;
  purpose: string;
  output: string[];
  status: string;
  tests: { input: string }[];
};
type Result = { input: string; pass: boolean; detail: string };

const BADGE: Record<string, string> = {
  approved: "text-emerald-400",
  experimental: "text-amber-300",
  proposed: "text-sky-300",
  deprecated: "text-dim",
  blocked: "text-rose-400",
};

/** Versioned skills: test them, approve them, roll them back. Nothing runs until it's approved. */
export function Skills() {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [results, setResults] = useState<Record<string, Result[] | string>>({});
  const [idea, setIdea] = useState("");
  const [busy, setBusy] = useState("");
  const [err, setErr] = useState("");

  const refresh = useCallback(() => void invoke<Skill[]>("skills").then(setSkills), []);
  useEffect(refresh, [refresh]);

  const key = (s: Skill) => `${s.id}@${s.version}`;
  const guard = async (what: string, run: () => Promise<unknown>) => {
    setBusy(what);
    setErr("");
    try {
      await run();
    } catch (e) {
      setErr(String(e));
    }
    setBusy("");
    refresh();
  };
  const setStatus = (s: Skill, status: string) => guard(key(s), () => invoke("skill_set_status", { id: s.id, version: s.version, status }));
  const test = (s: Skill) =>
    guard(key(s), async () => {
      const out = await invoke<Result[]>("skill_test", { id: s.id, version: s.version });
      setResults((r) => ({ ...r, [key(s)]: out }));
    });
  const run = (s: Skill) => {
    const input = window.prompt(`Input for ${s.id}`);
    if (!input) return;
    void guard(key(s), async () => {
      const out = await invoke("skill_run", { id: s.id, version: s.version, input });
      setResults((r) => ({ ...r, [key(s)]: JSON.stringify(out, null, 1) }));
    });
  };

  return (
    <div className="flex-1 select-text overflow-auto p-8">
      <section className="mb-8">
        <Label>Propose a skill</Label>
        <form
          className="flex gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (idea.trim()) void guard("propose", async () => { await invoke("propose_skill", { description: idea }); setIdea(""); });
          }}
        >
          <input
            className="flex-1 rounded-xl border border-line bg-raised px-4 py-2.5 outline-none focus:border-accent"
            placeholder="e.g. turn a topic into a 3-step cheat sheet"
            value={idea}
            onChange={(e) => setIdea(e.target.value)}
          />
          <button className="rounded-lg border border-line bg-raised px-4 text-sm hover:border-accent" disabled={busy === "propose"}>
            {busy === "propose" ? "drafting…" : "Draft"}
          </button>
        </form>
        <p className="mt-2 text-sm text-dim">Skills are prompt-only: they can ask the model and nothing else. A draft starts as “proposed”.</p>
        {err && <p className="mt-2 text-sm text-rose-400">{err}</p>}
      </section>

      <Label>Installed</Label>
      <ul className="space-y-3">
        {skills.map((s) => {
          const r = results[key(s)];
          return (
            <li key={key(s)} className="rounded-xl border border-line bg-raised p-4">
              <div className="flex items-baseline justify-between">
                <span className="font-semibold">{s.id} <span className="font-normal text-dim">v{s.version}</span></span>
                <span className={`text-sm ${BADGE[s.status] ?? ""}`}>{s.status}</span>
              </div>
              <p className="mt-1 text-sm text-dim">{s.purpose}</p>
              <div className="mt-3 flex flex-wrap gap-2 text-sm">
                {s.status === "proposed" && <Act onClick={() => setStatus(s, "experimental")}>Accept for testing</Act>}
                {(s.status === "experimental" || s.status === "approved") && <Act onClick={() => test(s)} busy={busy === key(s)}>Run tests ({s.tests.length})</Act>}
                {s.status === "experimental" && <Act onClick={() => setStatus(s, "approved")}>Approve</Act>}
                {s.status === "approved" && <Act onClick={() => run(s)}>Run…</Act>}
                {s.status === "approved" && <Act onClick={() => setStatus(s, "deprecated")}>Roll back</Act>}
                {s.status !== "blocked" && s.status !== "deprecated" && <Act onClick={() => setStatus(s, "blocked")}>Block</Act>}
              </div>
              {typeof r === "string" && <pre className="mt-3 whitespace-pre-wrap text-sm text-dim">{r}</pre>}
              {Array.isArray(r) && (
                <ul className="mt-3 space-y-1 text-sm">
                  {r.map((t, i) => (
                    <li key={i} className={t.pass ? "text-emerald-400" : "text-rose-400"}>
                      {t.pass ? "pass" : "fail"} · {t.input.slice(0, 50)} {t.detail && <span className="text-dim">({t.detail})</span>}
                    </li>
                  ))}
                </ul>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}

const Act = ({ busy, ...p }: React.ButtonHTMLAttributes<HTMLButtonElement> & { busy?: boolean }) => (
  <button {...p} disabled={busy} className="rounded-lg border border-line px-3 py-1.5 hover:border-accent disabled:opacity-50">
    {busy ? "working…" : p.children}
  </button>
);
