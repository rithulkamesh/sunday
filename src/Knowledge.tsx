import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { Label } from "./ui";

type Node = { name: string; subject: string; status: "new" | "learning" | "known"; needs: string[] };
type Doc = { id: number; title: string; chunks: number };

const TONE = {
  new: "border-line text-dim",
  learning: "border-amber-500/60 text-amber-300",
  known: "border-emerald-500/60 bg-emerald-500/10 text-emerald-300",
};

export const importNotes = async () => {
  const picked = await open({ multiple: true, filters: [{ name: "Notes", extensions: ["pdf", "md", "txt"] }] });
  return picked ? invoke<string[]>("import_notes", { paths: Array.isArray(picked) ? picked : [picked] }) : [];
};

/** Study material and the syllabus map: what you've imported, and what you know so far. */
export function Knowledge({ onAsk }: { onAsk: (text: string) => void }) {
  const [graph, setGraph] = useState<Node[]>([]);
  const [docs, setDocs] = useState<Doc[]>([]);
  const [status, setStatus] = useState<string[]>([]);
  const [hover, setHover] = useState<Node | null>(null);

  const refresh = useCallback(() => {
    void invoke<Node[]>("graph").then(setGraph);
    void invoke<Doc[]>("notes").then(setDocs);
  }, []);

  const doImport = useCallback(async (run: Promise<string[]>) => {
    setStatus(["importing…"]);
    setStatus(await run);
    refresh();
  }, [refresh]);

  useEffect(() => {
    refresh();
    // Drop files anywhere on the window.
    const drop = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "drop") void doImport(invoke<string[]>("import_notes", { paths: e.payload.paths }));
    });
    return () => void drop.then((off) => off());
  }, [refresh, doImport]);

  const subjects = [...new Set(graph.map((n) => n.subject))];
  const known = graph.filter((n) => n.status === "known").length;

  return (
    <div className="flex-1 select-text overflow-auto p-8">
      <section className="mb-10">
        <div className="mb-3 flex items-center justify-between">
          <Label>{`Notes · ${docs.length}`}</Label>
          <button className="rounded-lg border border-line bg-raised px-3 py-1.5 text-sm hover:border-accent" onClick={() => void doImport(importNotes())}>
            Import…
          </button>
        </div>
        {!docs.length && <p className="text-sm text-dim">Drop PDFs, Markdown or text files here. Answers will cite them, and quizzes draw on them.</p>}
        <ul className="space-y-1.5">
          {docs.map((d) => (
            <li key={d.id} className="flex items-center justify-between rounded-lg border border-line bg-raised px-4 py-2 text-sm">
              <span>{d.title}</span>
              <span className="flex items-center gap-4 text-dim">
                {d.chunks} passages
                <button className="hover:text-ink" onClick={() => void invoke("remove_note", { id: d.id }).then(refresh)}>
                  remove
                </button>
              </span>
            </li>
          ))}
        </ul>
        {status.map((s, i) => (
          <p key={i} className="mt-2 text-sm text-dim">{s}</p>
        ))}
      </section>

      <section>
        <Label>{`Syllabus · ${known} of ${graph.length} known`}</Label>
        <p className="mb-4 h-5 text-sm text-dim">
          {hover ? (hover.needs.length ? `${hover.name} builds on: ${hover.needs.join(", ")}` : `${hover.name} has no prerequisites`) : "Click a concept to be quizzed on it. A concept is known after two correct answers in a row."}
        </p>
        {subjects.map((s) => (
          <div key={s} className="mb-5">
            <h3 className="mb-2 text-sm font-semibold">{s}</h3>
            <div className="flex flex-wrap gap-1.5">
              {graph.filter((n) => n.subject === s).map((n) => (
                <button
                  key={n.name}
                  onMouseEnter={() => setHover(n)}
                  onMouseLeave={() => setHover(null)}
                  onClick={() => onAsk(`quiz me on ${n.name}`)}
                  className={`rounded-full border px-2.5 py-1 text-xs hover:brightness-125 ${TONE[n.status]}`}
                >
                  {n.name}
                </button>
              ))}
            </div>
          </div>
        ))}
      </section>
    </div>
  );
}
