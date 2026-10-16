import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Label } from "./ui";

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
};

/** Models and a health check: when something goes wrong, which part is it? */
export function Settings() {
  const [d, setD] = useState<Diag | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const refresh = () => {
    void invoke<Diag>("diagnostics").then(setD);
    void invoke<string[]>("models").then(setModels);
  };
  useEffect(refresh, []);

  const Row = ({ ok, name, note }: { ok: boolean; name: string; note?: string }) => (
    <li className="flex justify-between border-b border-line py-2 text-sm">
      <span>{name}</span>
      <span className={ok ? "text-emerald-400" : "text-rose-400"}>{ok ? "ok" : note ?? "missing"}</span>
    </li>
  );

  return (
    <div className="flex-1 select-text overflow-auto p-8">
      <Label>Model</Label>
      <div className="mb-8 flex items-center gap-3">
        <select
          className="rounded-lg border border-line bg-raised px-3 py-2 text-sm"
          value={d?.model ?? ""}
          onChange={(e) => void invoke("set_model", { name: e.target.value }).then(refresh)}
        >
          {[...new Set([d?.model ?? "", ...models])].filter(Boolean).map((m) => (
            <option key={m}>{m}</option>
          ))}
        </select>
        <span className="text-sm text-dim">Used for chat, quizzes, grading and skills. Smaller is faster.</span>
      </div>

      <Label>Diagnostics</Label>
      {d && (
        <ul className="max-w-xl">
          <Row ok={d.ollama} name="Ollama running" note="not reachable: start Ollama" />
          <Row ok={d.model_installed} name={`Model ${d.model}`} note={`run: ollama pull ${d.model}`} />
          <Row ok={d.vision_installed} name={`Vision model ${d.vision_model} (screen)`} note={`run: ollama pull ${d.vision_model}`} />
          <Row ok={d.ocr} name="Screen text reader (exact pointing)" note="run scripts/build-ocr.sh" />
          <Row ok={d.stt_models} name="Speech models" note="run scripts/fetch-models.sh" />
          <Row ok={d.concepts > 0} name={`Syllabus · ${d.concepts} concepts`} />
          <Row ok name={`${d.notes} notes imported · ${d.skills} skills`} />
        </ul>
      )}
      <button className="mt-4 rounded-lg border border-line bg-raised px-3 py-1.5 text-sm hover:border-accent" onClick={refresh}>
        Re-check
      </button>
    </div>
  );
}
