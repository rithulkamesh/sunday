import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Providers } from "./Providers";

type Diag = {
  ollama: boolean;
  model: string;
  model_installed: boolean;
  vision_model: string;
  vision_installed: boolean;
  concepts: number;
  notes: number;
  skills: number;
  stt_models: boolean;
  ocr: boolean;
  embed_model: string;
  embed_installed: boolean;
  facts: number;
};

type Flags = { web: boolean; learn_chat: boolean; chrome: string | null };
type Voice = { enrolled: boolean; model: boolean; target: number };
type VoiceEvent = { state: "enrolling" | "enrolled" | "cancelled" | "cleared" | "error"; secs?: number; target?: number; message?: string };

/** Models and a health check: when something goes wrong, which part is it? */
export function Settings() {
  const [d, setD] = useState<Diag | null>(null);
  const [voice, setVoice] = useState<Voice | null>(null);
  const [ev, setEv] = useState<VoiceEvent | null>(null);
  const [flags, setFlags] = useState<Flags | null>(null);
  const loadFlags = () => void invoke<Flags>("flags").then(setFlags);
  const setFlag = (key: string, on: boolean) => void invoke("set_flag", { key, on }).then(loadFlags);
  useEffect(loadFlags, []);
  const refresh = () => {
    void invoke<Diag>("diagnostics").then(setD);
  };
  useEffect(refresh, []);

  useEffect(() => {
    const status = () => void invoke<Voice>("voice_status").then(setVoice);
    status();
    const sub = listen<VoiceEvent>("voice", (e) => {
      setEv(e.payload);
      if (e.payload.state !== "enrolling") status();
    });
    return () => void sub.then((off) => off());
  }, []);
  const enrolling = ev?.state === "enrolling";

  const Row = ({ ok, name, note }: { ok: boolean; name: string; note?: string }) => (
    <li className="flex justify-between border-b border-line py-2 text-sm">
      <span>{name}</span>
      <span className={ok ? "text-emerald-400" : "text-rose-400"}>{ok ? "ok" : note ?? "missing"}</span>
    </li>
  );

  const tabs = [
    ["models", "Models", "Which model answers each job, and where your prompts go."],
    ["privacy", "Privacy", "What Sunday may do beyond this Mac."],
    ["voice", "Your voice", "So Sunday answers to you and no one else."],
    ["health", "Health", "When something goes wrong, which part is it?"],
  ] as const;
  const [tab, setTab] = useState<(typeof tabs)[number][0]>("models");
  const [, title, sub] = tabs.find((t) => t[0] === tab)!;

  return (
    <div className="flex min-h-0 flex-1 select-text">
      <nav aria-label="Settings" className="w-48 shrink-0 border-r border-line p-4">
        {tabs.map(([id, name]) => (
          <button
            key={id}
            aria-current={tab === id ? "page" : undefined}
            className={`mb-1 block h-9 w-full rounded-lg px-3 text-left text-sm focus-visible:outline-2 focus-visible:outline-accent ${tab === id ? "bg-raised text-ink" : "text-dim hover:text-ink"}`}
            onClick={() => setTab(id)}
          >
            {name}
          </button>
        ))}
      </nav>
      <div className="flex-1 overflow-auto p-8">
        <h2 className="text-lg font-semibold">{title}</h2>
        <p className="mb-6 text-sm text-dim">{sub}</p>

        {tab === "models" && <Providers />}

        {tab === "privacy" && (
          <>
          <div className="mb-8 max-w-xl space-y-3 text-sm">
            <label className="flex items-start gap-3">
              <input type="checkbox" className="mt-1" checked={flags?.web ?? false} onChange={(e) => setFlag("web", e.target.checked)} />
              <span>
                Allow web access
                <span className="block text-dim">
                  Lets Sunday read pages and search the web when you ask. Nothing is fetched otherwise. Pages that need JavaScript use headless Chrome
                  {flags?.chrome ? "" : " (none found: install Google Chrome for those)"}.
                </span>
              </span>
            </label>
            <label className="flex items-start gap-3">
              <input type="checkbox" className="mt-1" checked={flags?.learn_chat ?? true} onChange={(e) => setFlag("learn_chat", e.target.checked)} />
              <span>
                Learn from conversations
                <span className="block text-dim">When you say goodbye, lasting facts about you are saved as unconfirmed guesses you can review in Memory.</span>
              </span>
            </label>
          </div>
          </>
        )}

        {tab === "voice" && (
          <>
          <div className="mb-8 max-w-xl">
            <p className="mb-3 text-sm text-dim">
              {voice?.enrolled ? "Sunday only answers to your voice." : "Not learned yet, so Sunday answers anyone it hears."} Learning takes about {voice?.target ?? 20} seconds of you talking.
            </p>
            {enrolling && (
              <div className="mb-3">
                <div className="h-1.5 overflow-hidden rounded bg-track">
                  <div className="h-full bg-accent transition-all" style={{ width: `${Math.min(100, ((ev?.secs ?? 0) / (ev?.target ?? 20)) * 100)}%` }} />
                </div>
                <p className="mt-2 text-sm text-dim">Keep talking: read something aloud or describe your day.</p>
              </div>
            )}
            {ev?.state === "error" && <p className="mb-3 text-sm text-rose-400">{ev.message}</p>}
            <div className="flex gap-2 text-sm">
              {enrolling ? (
                <button className="rounded-lg border border-line bg-raised px-3 py-1.5 hover:border-accent" onClick={() => void invoke("cancel_enroll")}>Cancel</button>
              ) : (
                <button className="rounded-lg border border-line bg-raised px-3 py-1.5 hover:border-accent disabled:opacity-50" disabled={voice?.model === false} onClick={() => void invoke("enroll_voice")}>
                  {voice?.enrolled ? "Learn again" : "Learn my voice"}
                </button>
              )}
              {voice?.enrolled && !enrolling && (
                <button className="rounded-lg border border-line px-3 py-1.5 hover:border-accent" onClick={() => void invoke("forget_voice")}>Forget</button>
              )}
            </div>
            {voice?.model === false && <p className="mt-2 text-sm text-rose-400">Speaker model missing: run scripts/fetch-models.sh</p>}
          </div>
          </>
        )}

        {tab === "health" && (
          <>
          {d && (
            <ul className="max-w-xl">
              <Row ok={d.ollama} name="Ollama running" note="not reachable: start Ollama" />
              <Row ok={d.model_installed} name={`Model ${d.model}`} note={`run: ollama pull ${d.model}`} />
              <Row ok={d.vision_installed} name={`Vision model ${d.vision_model} (screen)`} note={`run: ollama pull ${d.vision_model}`} />
              <Row ok={d.embed_installed} name={`Memory search model ${d.embed_model} · ${d.facts} facts`} note={`run: ollama pull ${d.embed_model}`} />
              <Row ok={d.ocr} name="Screen text reader (exact pointing)" note="run scripts/build-ocr.sh" />
              <Row ok={d.stt_models} name="Speech models" note="run scripts/fetch-models.sh" />
              <Row ok={d.concepts > 0} name={`Learning map · ${d.concepts} concepts`} />
              <Row ok name={`${d.notes} notes imported · ${d.skills} skills`} />
            </ul>
          )}
          <button className="mt-4 rounded-lg border border-line bg-raised px-3 py-1.5 text-sm hover:border-accent" onClick={refresh}>
            Re-check
          </button>
          </>
        )}

      </div>
    </div>
  );
}
