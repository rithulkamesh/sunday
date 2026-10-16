import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

/** Main window: the big surface for the learning system. Starts as a transcript log. */
export function Workspace() {
  const [log, setLog] = useState<string[]>([]);

  useEffect(() => {
    const sub = listen<string>("utterance", (e) =>
      e.payload && setLog((l) => [...l, e.payload]),
    );
    return () => void sub.then((off) => off());
  }, []);

  return (
    <div className="flex h-full bg-canvas">
      <aside className="w-56 shrink-0 border-r border-line bg-panel p-5">
        <h1 className="text-lg font-semibold">Sunday</h1>
        <p className="mt-1 text-sm text-dim">Hold fn or say “sunday”</p>
      </aside>
      <main className="flex-1 overflow-auto p-8">
        <h2 className="mb-4 font-mono text-xs uppercase tracking-widest text-dim">Transcript</h2>
        <ul className="space-y-2">
          {log.map((t, i) => (
            <li key={i} className="rounded-lg border border-line bg-raised px-4 py-2.5">
              {t}
            </li>
          ))}
        </ul>
      </main>
    </div>
  );
}
