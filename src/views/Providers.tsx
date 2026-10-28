import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Provider = {
  id: string;
  label: string;
  local: boolean;
  suggested_model: string;
  key_var: string | null;
  key_set: boolean;
  key_from_env: boolean;
  conf: { var: string; label: string; placeholder: string; value: string | null } | null;
};
type Choice = { provider: string | null; model: string };
type Embed = { provider: "ollama" | "local"; model: string; local_installed: boolean };
type Data = { roles: { chat: Choice; planner: Choice }; embed: Embed; providers: Provider[] };
type Role = "chat" | "planner";
type Form = { provider: string; model: string; key: string; conf: string };

const ROLES: { id: Role; name: string; does: string }[] = [
  { id: "chat", name: "Chat", does: "Replies, quizzes, grading and skills" },
  { id: "planner", name: "Planner", does: "Picks the next step when Sunday acts on your computer" },
];
const FIXED = [
  { name: "Vision", does: "Reads diagrams and non-text screens", value: "Ollama" },
  { name: "Router", does: "Decides what you meant", value: "Built-in rules" },
];

const ring = "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent";
const btn = `h-9 rounded-lg border border-line bg-raised px-4 text-sm hover:border-accent disabled:opacity-50 ${ring}`;
const primary = `h-9 rounded-lg bg-ink px-4 text-sm font-semibold text-canvas hover:bg-white/90 disabled:opacity-50 ${ring}`;
const input = `h-10 w-full rounded-lg border border-line bg-canvas px-3 text-sm ${ring} aria-[invalid=true]:border-rose-400`;

/** Where a job's prompts go: on this Mac or off it. */
function Where({ local }: { local: boolean }) {
  return (
    <span className={`inline-flex items-center gap-1.5 text-xs ${local ? "text-emerald-400" : "text-amber-400"}`}>
      <span aria-hidden className="size-1.5 rounded-full bg-current" />
      {local ? "On this Mac" : "Leaves this Mac"}
    </span>
  );
}


const EMBEDDERS = [
  { id: "ollama", label: "Ollama", model: "nomic-embed-text-v2-moe", note: "Needs Ollama running." },
  { id: "local", label: "On this Mac", model: "all-MiniLM-L6-v2", note: "No Ollama. About 90 MB, English." },
] as const;

/** Memory and note search: where the embeddings come from. Tried before it is saved. */
function Embeddings({ embed, reload, done }: { embed: Embed; reload: () => Promise<void>; done: (t: string) => void }) {
  const [open, setOpen] = useState(false);
  const [pick, setPick] = useState<Embed["provider"]>(embed.provider);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const current = EMBEDDERS.find((e) => e.id === embed.provider)!;
  const chosen = EMBEDDERS.find((e) => e.id === pick)!;
  const submit = async () => {
    setBusy(true);
    setErr("");
    try {
      await invoke("set_embedder", { id: pick });
      await reload();
      done(`Memory search now uses ${chosen.label} (${chosen.model}). Facts saved with the other one are found by keyword until they come up again.`);
      setOpen(false);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <li className="px-5 py-4">
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0">
          <h3 className="text-sm font-semibold">Embeddings</h3>
          <p className="text-xs text-dim">Memory and note search</p>
        </div>
        <div className="flex shrink-0 items-center gap-4">
          <div className="text-right">
            <p className="font-mono text-sm">{current.label} <span className="text-dim">{current.model}</span></p>
            <p className="text-xs"><Where local /></p>
          </div>
          <button className={btn} aria-expanded={open} onClick={() => (setPick(embed.provider), setErr(""), setOpen(!open))}>{open ? "Close" : "Change"}</button>
        </div>
      </div>
      {open && (
        <form className="mt-4 space-y-4 border-t border-line pt-5" onSubmit={(e) => (e.preventDefault(), void submit())} onKeyDown={(e) => e.key === "Escape" && setOpen(false)} aria-label="Change embeddings">
          <fieldset>
            <legend className="mb-2 text-sm">Embedder</legend>
            <div className="grid gap-2 min-[560px]:grid-cols-2">
              {EMBEDDERS.map((o) => (
                <label key={o.id} className="flex cursor-pointer flex-col gap-1 rounded-lg border border-line px-3 py-2.5 text-sm has-[:checked]:border-accent has-[:checked]:bg-raised has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-accent">
                  <span className="flex items-center gap-2">
                    <input type="radio" name="embedder" className="accent-white" autoFocus={o.id === pick} checked={pick === o.id} onChange={() => (setPick(o.id), setErr(""))} />
                    {o.label}
                    <span className="font-mono text-xs text-dim">{o.model}</span>
                  </span>
                  <span className="pl-6 text-xs text-dim">{o.note}</span>
                </label>
              ))}
            </div>
          </fieldset>
          {pick === "local" && !embed.local_installed && <p className="text-xs text-dim">Not downloaded yet. In a terminal: scripts/fetch-embed-model.sh</p>}
          {err && <p role="alert" className="text-sm text-rose-400">{err}</p>}
          <div className="flex gap-2">
            <button type="submit" className={primary} disabled={busy}>{busy ? "Testing…" : "Test and use"}</button>
            <button type="button" className={btn} onClick={() => setOpen(false)} disabled={busy}>Cancel</button>
          </div>
        </form>
      )}
    </li>
  );
}

/** Which provider answers which job. One job is changed at a time, and only saved once it works. */
export function Providers() {
  const [data, setData] = useState<Data | null>(null);
  const [editing, setEditing] = useState<Role | null>(null);
  const [form, setForm] = useState<Form | null>(null);
  const [models, setModels] = useState<Record<string, string[]>>({});
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<{ field?: "key" | "model"; text: string } | null>(null);
  const [done, setDone] = useState("");
  const opener = useRef<HTMLButtonElement | null>(null);

  const load = () => invoke<Data>("provider_settings").then((d) => setData(d));
  useEffect(() => void load(), []);
  useEffect(() => {
    if (!done) return;
    const t = setTimeout(() => setDone(""), 5000);
    return () => clearTimeout(t);
  }, [done]);

  if (!data) return null;
  const byId = (id: string) => data.providers.find((p) => p.id === id)!;
  const current = (r: Role) => ({ provider: data.roles[r].provider ?? data.roles.chat.provider ?? "ollama", model: data.roles[r].model });
  const fetchModels = (id: string) => void invoke<string[]>("provider_models", { id }).then((m) => setModels((all) => ({ ...all, [id]: m })));

  const open = (r: Role, el: HTMLButtonElement) => {
    const c = current(r);
    opener.current = el;
    setEditing(r);
    setErr(null);
    setForm({ ...c, key: "", conf: byId(c.provider).conf?.value ?? "" });
    fetchModels(c.provider);
  };
  const close = () => {
    setEditing(null);
    opener.current?.focus();
  };
  const pick = (id: string) => {
    const p = byId(id);
    setErr(null);
    setForm({ provider: id, model: id === current(editing!).provider ? current(editing!).model : p.suggested_model, key: "", conf: p.conf?.value ?? "" });
    fetchModels(id);
  };

  const submit = async () => {
    if (!form || !editing) return;
    const p = byId(form.provider);
    const bad = (field: "key" | "model", text: string) => (setErr({ field, text }), document.getElementById(field)?.focus());
    if (p.key_var && !p.key_set && !p.key_from_env && !form.key.trim()) return bad("key", `${p.label} needs an API key.`);
    if (!form.model.trim()) return bad("model", "Choose a model.");
    setBusy(true);
    setErr(null);
    try {
      if (form.key.trim()) await invoke("set_provider_secret", { var: p.key_var, value: form.key });
      if (p.conf && form.conf !== (p.conf.value ?? "")) await invoke("set_provider_conf", { var: p.conf.var, value: form.conf });
      if (p.id !== "ollama") await invoke("test_provider", { id: p.id, model: form.model.trim() });
      await invoke("set_role", { role: editing, provider: p.id, model: form.model });
      await load();
      setDone(`${ROLES.find((r) => r.id === editing)!.name} now uses ${p.label} (${form.model.trim()}).`);
      close();
    } catch (e) {
      setErr({ text: `${String(e)}${form.key.trim() ? " Your key was saved; nothing else changed." : " Nothing was changed."}` });
      await load();
    } finally {
      setBusy(false);
    }
  };
  const removeKey = async (p: Provider) => {
    await invoke("set_provider_secret", { var: p.key_var, value: null });
    await load();
    setErr(null);
  };

  const editor = () => {
    const p = byId(form!.provider);
    return (
      <form
        className="mt-4 space-y-5 border-t border-line pt-5"
        onSubmit={(e) => (e.preventDefault(), void submit())}
        onKeyDown={(e) => e.key === "Escape" && close()}
        aria-label={`Change ${editing}`}
      >
        <fieldset>
          <legend className="mb-2 text-sm">Provider</legend>
          <div className="grid gap-2 min-[560px]:grid-cols-2">
            {data.providers.map((o) => (
              <label key={o.id} className="flex cursor-pointer items-center justify-between gap-2 rounded-lg border border-line px-3 py-2.5 text-sm has-[:checked]:border-accent has-[:checked]:bg-raised has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-accent">
                <span className="flex items-center gap-2">
                  <input type="radio" name="provider" className="accent-white" autoFocus={o.id === form!.provider} checked={form!.provider === o.id} onChange={() => pick(o.id)} />
                  {o.label}
                </span>
                <span className={`shrink-0 whitespace-nowrap text-xs ${o.local ? "text-emerald-400" : "text-dim"}`}>{o.local ? "On this Mac" : o.key_set ? "Key saved" : "Needs key"}</span>
              </label>
            ))}
          </div>
        </fieldset>

        {p.key_var && (
          <div>
            <label htmlFor="key" className="mb-1.5 block text-sm">API key</label>
            {p.key_from_env ? (
              <p className="text-sm text-dim">Using {p.key_var} from the environment.</p>
            ) : (
              <>
                <input id="key" type="password" autoComplete="off" spellCheck={false} className={`${input} font-mono`} aria-invalid={err?.field === "key"} aria-describedby="key-help" value={form!.key} onChange={(e) => setForm({ ...form!, key: e.target.value })} placeholder={p.key_set ? "••••••••  saved" : ""} />
                <p id="key-help" className="mt-1.5 text-xs text-dim">
                  {p.key_set ? "A key is saved. Leave this empty to keep it." : "Stored in your Keychain. Sunday can't show it again."}
                  {p.key_set && (
                    <button type="button" className={`ml-2 underline ${ring}`} onClick={() => void removeKey(p)}>Remove saved key</button>
                  )}
                </p>
                {err?.field === "key" && <p role="alert" className="mt-1.5 text-sm text-rose-400">{err.text}</p>}
              </>
            )}
          </div>
        )}

        {p.conf && (
          <div>
            <label htmlFor="conf" className="mb-1.5 block text-sm">{p.conf.label}</label>
            <input id="conf" className={`${input} font-mono`} value={form!.conf} placeholder={p.conf.placeholder} onChange={(e) => setForm({ ...form!, conf: e.target.value })} />
            <p className="mt-1.5 text-xs text-dim">Leave empty for {p.conf.placeholder}.</p>
          </div>
        )}

        <div>
          <label htmlFor="model" className="mb-1.5 block text-sm">Model</label>
          <input id="model" list="models" spellCheck={false} className={`${input} font-mono`} aria-invalid={err?.field === "model"} aria-describedby="model-help" value={form!.model} onChange={(e) => setForm({ ...form!, model: e.target.value })} />
          <datalist id="models">{(models[p.id] ?? []).map((m) => <option key={m} value={m} />)}</datalist>
          <p id="model-help" className="mt-1.5 text-xs text-dim">
            {p.id === "mlx" && !(models.mlx ?? []).includes(form!.model)
              ? `Not downloaded yet. In a terminal: scripts/fetch-mlx-model.sh ${form!.model}`
              : models[p.id]?.length ? `${models[p.id].length} models found. Type to filter.` : `Suggested: ${p.suggested_model}.`}
          </p>
          {err?.field === "model" && <p role="alert" className="mt-1.5 text-sm text-rose-400">{err.text}</p>}
        </div>

        {!p.local && (
          <p className="rounded-lg border border-amber-400/30 bg-amber-400/5 px-3 py-2 text-sm text-amber-200">
            Prompts for this job go to {p.label}, including notes and memories that match your question.
          </p>
        )}
        {err && !err.field && <p role="alert" className="text-sm text-rose-400">{err.text}</p>}

        <div className="flex gap-2">
          <button type="submit" className={primary} disabled={busy}>{busy ? "Testing…" : p.id === "ollama" ? "Use this" : "Test and use"}</button>
          <button type="button" className={btn} onClick={close} disabled={busy}>Cancel</button>
        </div>
      </form>
    );
  };

  return (
    <div className="max-w-2xl">
      <p role="status" aria-live="polite" className="min-h-6 text-sm text-emerald-400">{done}</p>
      <ul className="divide-y divide-line rounded-xl border border-line bg-panel">
        {ROLES.map((r) => {
          const c = current(r.id);
          const p = byId(c.provider);
          const open_ = editing === r.id;
          return (
            <li key={r.id} className="px-5 py-4">
              <div className="flex items-center justify-between gap-4">
                <div className="min-w-0">
                  <h3 className="text-sm font-semibold">{r.name}</h3>
                  <p className="text-xs text-dim">{r.does}</p>
                </div>
                <div className="flex shrink-0 items-center gap-4">
                  <div className="text-right">
                    <p className="font-mono text-sm">
                      {p.label} <span className="text-dim">{c.model}</span>
                    </p>
                    <p className="flex justify-end gap-2 text-xs">
                      <Where local={p.local} />
                      {r.id === "planner" && !data.roles.planner.provider && <span className="text-dim">Follows Chat</span>}
                    </p>
                  </div>
                  <button className={btn} aria-expanded={open_} onClick={(e) => (open_ ? close() : open(r.id, e.currentTarget))}>{open_ ? "Close" : "Change"}</button>
                </div>
              </div>
              {open_ && form && editor()}
            </li>
          );
        })}
        <Embeddings embed={data.embed} reload={load} done={setDone} />
        {FIXED.map((f) => (
          <li key={f.name} className="flex items-center justify-between gap-4 px-5 py-4">
            <div className="min-w-0">
              <h3 className="text-sm font-semibold">{f.name}</h3>
              <p className="text-xs text-dim">{f.does}</p>
            </div>
            <div className="text-right">
              <p className="font-mono text-sm">{f.value}</p>
              <p className="text-xs"><Where local /></p>
            </div>
          </li>
        ))}
      </ul>
      <p className="mt-3 text-xs text-dim">Vision, embeddings and the router always run on this Mac, so screenshots and notes are never sent out.</p>
    </div>
  );
}
