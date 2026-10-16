import { ThinkingOrb } from "thinking-orbs";

/** The pill's status orb: rolling waveform while listening, orbiting dots while transcribing. */
export function Status({ thinking }: { thinking: boolean }) {
  return <ThinkingOrb state={thinking ? "working" : "listening"} size={20} theme="dark" />;
}

/** Reveals the transcript word by word; re-keyed on new text so it replays. */
export function Words({ text }: { text: string }) {
  const words = text.split(/\s+/).filter(Boolean);
  return (
    <p className="max-w-[460px] text-base leading-snug text-ink">
      {words.map((w, i) => (
        <span
          key={i}
          className="mr-[0.3em] inline-block animate-word"
        >
          {w}
        </span>
      ))}
    </p>
  );
}

export const Label = ({ children }: { children: string }) => (
  <h2 className="mb-2 font-mono text-xs uppercase tracking-widest text-dim">{children}</h2>
);
