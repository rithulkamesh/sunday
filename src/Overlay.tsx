import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

type Mark = { kind: "box" | "circle" | "underline" | "arrow"; x: number; y: number; w: number; h: number; label: string };

const INK = "#7dd3fc"; // cyan, with a violet glow
const GLOW = "drop-shadow(0 0 6px #a78bfa) drop-shadow(0 0 14px #7dd3fc66)";

/** Click-through layer over the screen: draws what Sunday is pointing at while it explains. */
export function Overlay() {
  const [marks, setMarks] = useState<Mark[]>([]);
  const [size, setSize] = useState({ w: window.innerWidth, h: window.innerHeight });

  useEffect(() => {
    const onResize = () => setSize({ w: window.innerWidth, h: window.innerHeight });
    window.addEventListener("resize", onResize);
    const sub = listen<{ marks: Mark[] }>("overlay", (e) => setMarks(e.payload.marks));
    return () => {
      window.removeEventListener("resize", onResize);
      void sub.then((off) => off());
    };
  }, []);

  // Marks arrive as 0..1000 of the screenshot; the overlay covers the same display.
  const px = (m: Mark) => ({ x: (m.x / 1000) * size.w, y: (m.y / 1000) * size.h, w: (m.w / 1000) * size.w, h: (m.h / 1000) * size.h });

  return (
    <svg className="h-full w-full" width={size.w} height={size.h} style={{ filter: GLOW }}>
      <defs>
        <marker id="head" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
          <path d="M0 0 L10 5 L0 10 z" fill={INK} />
        </marker>
      </defs>
      {marks.map((m, i) => {
        const r = px(m);
        const delay = `${i * 220}ms`;
        const common = { stroke: INK, strokeWidth: 3, fill: "none", className: "mark", style: { animationDelay: delay } };
        // The arrow comes in from the upper left and stops at the item's edge.
        const cx = r.x + r.w / 2, cy = r.y + r.h / 2;
        const tail = { x: Math.max(20, cx - 170), y: Math.max(20, cy - 120) };
        // The label sits above its mark, or below it near the top edge, so it never covers what it points at.
        const above = r.y >= 44;
        const ly = above ? r.y - 34 : r.y + r.h + 10;
        return (
          <g key={i}>
            {m.kind === "box" && <rect x={r.x - 4} y={r.y - 4} width={r.w + 8} height={r.h + 8} rx={10} {...common} fill="#7dd3fc14" />}
            {m.kind === "circle" && <ellipse cx={cx} cy={cy} rx={r.w / 2 + 8} ry={r.h / 2 + 8} {...common} />}
            {m.kind === "underline" && <line x1={r.x} y1={r.y + r.h + 4} x2={r.x + r.w} y2={r.y + r.h + 4} {...common} strokeWidth={4} strokeLinecap="round" />}
            {m.kind === "arrow" && <line x1={tail.x} y1={tail.y} x2={r.x + r.w * 0.1} y2={r.y + r.h * 0.1} {...common} strokeLinecap="round" markerEnd="url(#head)" />}
            {m.label && (
              <g className="mark" style={{ animationDelay: delay }}>
                <rect x={Math.min(r.x, size.w - 200)} y={ly} width={m.label.length * 8.4 + 20} height={26} rx={13} fill="#0a0a0acc" stroke={INK} />
                <text x={Math.min(r.x, size.w - 200) + 10} y={ly + 18} fill="#fff" fontSize="14" fontFamily="IBM Plex Sans, system-ui">
                  {m.label}
                </text>
              </g>
            )}
          </g>
        );
      })}
    </svg>
  );
}
