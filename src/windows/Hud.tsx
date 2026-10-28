import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { VoiceBeam } from "voice-glow";
import { Status, Words } from "../ui";

type State = "idle" | "listening" | "thinking";

export function Hud() {
  const [state, setState] = useState<State>("idle");
  const [text, setText] = useState("");
  const [open, setOpen] = useState(false);
  const level = useRef(0);
  const getLevel = useRef(() => level.current).current;

  useEffect(() => {
    const subs = [
      listen<number>("level", (e) => (level.current = e.payload)),
      listen<State>("state", (e) => {
        setState(e.payload);
        setOpen(e.payload !== "idle");
        if (e.payload === "thinking") setText(""); // spinner only until the reply is audible
        // Let the exit animation finish before the words disappear.
        if (e.payload === "idle") setTimeout(() => setText(""), 250);
      }),
      listen<string>("transcript", (e) => setText(e.payload)),
      // Reply sentences arrive as they start playing.
      listen<string>("say", (e) => {
        setState("listening");
        setText((t) => `${t} ${e.payload}`.trim());
      }),
    ];
    return () => void subs.forEach((s) => s.then((off) => off()));
  }, []);

  return (
    <main className="flex h-full items-end justify-center pb-1">
      <div
        className={`transition-all duration-200 ease-out ${
          open ? "translate-y-0 scale-100 opacity-100" : "translate-y-4 scale-90 opacity-0"
        }`}
      >
        <VoiceBeam
          type="pill"
          colorVariant="mono"
          level={state === "listening" ? getLevel : 0}
          strength={0.5}
          bloomOpacity={0.6}
          idle={0.1}
        >
          <div className="flex items-center gap-4 rounded-[28px] bg-black/45 px-6 backdrop-blur-2xl py-3.5">
            <Status thinking={state === "thinking"} />
            {text && <Words text={text} />}
          </div>
        </VoiceBeam>
      </div>
    </main>
  );
}
