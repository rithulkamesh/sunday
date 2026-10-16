import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Label } from "./ui";

type Fact = { id: number; ts: number; kind: "user" | "web" | "chat"; source: string; title: string; text: string; confidence: number; event_date: string; score: number };

const KIND = {
  user: { label: "you told me", tone: "text-emerald-400" },
  web: { label: "read on the web", tone: "text-sky-300" },
  chat: { label: "inferred, unconfirmed", tone: "text-amber-300" },
} as const;

const when = (ts: number) => new Date(ts * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short" });
const host = (s: string) => s.replace(/^https?:\/\//, "").replace(/^www\./, "").split("/")[0];

/** What Sunday remembers, and where each thing came from. Everything here can be deleted. */
export function Memory({ onAsk }: { onAsk: (text: string) => void }) {
  const [facts, setFacts] = useState<Fact[]>([]);
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<"all" | Fact["kind"]>("all");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(() => {
    if (query.trim()) {
      setBusy(true);
      void invoke<Fact[]>("memory_search", { query }).then(setFacts).finally(() => setBusy(false));
    } else void invoke<Fact[]>("memory_list", { limit: 500 }).then(setFacts);
  }, [query]);

  useEffect(refresh, [refresh]);

  useEffect(() => {
    const sub = listen<{ kind: string; added: number }>("memory", (e) => {
      setNote(e.payload.added ? `Saved ${e.payload.added} new ${e.payload.added === 1 ? "fact" : "facts"} (${e.payload.kind}).` : "Nothing new to save: already known.");
      refresh();
    });
    return () => void sub.then((off) => off());
  }, [refresh]);

  const shown = facts.filter((f) => kind === "all" || f.kind === kind);
  const sources = [...new Set(facts.filter((f) => f.kind === "web").map((f) => f.source))];

  return (
    <div className="flex-1 select-text overflow-auto p-8">
      <Label>{`Memory · ${facts.length}`}</Label>
      <p className="mb-4 text-sm text-dim">
        Say “remember that …”, “learn this page” (with a browser in front), “read https://…” or “research …”. Web access must be on in Settings.
      </p>
      <div className="mb-4 flex flex-wrap items-center gap-3">
        <input
          className="min-w-[260px] flex-1 rounded-xl border border-line bg-raised px-4 py-2 text-sm outline-none focus:border-accent"
          placeholder="Search by meaning…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        {(["all", "user", "web", "chat"] as const).map((k) => (
          <button key={k} onClick={() => setKind(k)} className={`rounded-full border px-3 py-1 text-xs ${kind === k ? "border-accent text-ink" : "border-line text-dim hover:text-ink"}`}>
            {k === "all" ? "all" : KIND[k].label}
          </button>
        ))}
      </div>
      {note && <p className="mb-3 text-sm text-emerald-400">{note}</p>}
      {busy && <p className="mb-3 text-sm text-dim">searching…</p>}
      {!shown.length && !busy && <p className="text-sm text-dim">{query ? "Nothing relevant." : "Nothing remembered yet."}</p>}
      <ul className="space-y-2">
        {shown.map((f) => (
          <li key={f.id} className="rounded-xl border border-line bg-raised p-4">
            <p className="leading-snug">{f.text}</p>
            <p className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-dim">
              <span className={KIND[f.kind]?.tone}>{KIND[f.kind]?.label ?? f.kind}</span>
              {f.kind === "web" && <span title={f.source}>{host(f.source)}</span>}
              <span>{when(f.ts)}</span>
              {f.confidence < 1 && <span>{Math.round(f.confidence * 100)}% sure</span>}
              {f.event_date && <span>about {f.event_date}</span>}
              {f.score > 0 && <span>match {f.score.toFixed(2)}</span>}
              <button className="ml-auto hover:text-ink" onClick={() => void invoke("memory_delete", { id: f.id }).then(refresh)}>
                forget
              </button>
            </p>
          </li>
        ))}
      </ul>

      {!!sources.length && (
        <section className="mt-8">
          <Label>Pages read</Label>
          <ul className="space-y-1 text-sm">
            {sources.map((s) => (
              <li key={s} className="flex justify-between gap-4">
                <span className="truncate" title={s}>{host(s)} <span className="text-dim">{s.length > 40 ? "…" + s.slice(-30) : s}</span></span>
                <button className="text-dim hover:text-ink" onClick={() => void invoke("memory_delete_source", { source: s }).then(refresh)}>
                  forget this page
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
      <button className="mt-8 text-sm text-dim hover:text-ink" onClick={() => onAsk("what do you remember about me?")}>
        Ask Sunday what it remembers
      </button>
    </div>
  );
}
