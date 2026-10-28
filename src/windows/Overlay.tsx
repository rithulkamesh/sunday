import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

type Box = { x: number; y: number; w: number; h: number };
type Mark = Box & { kind: "box" | "circle" | "underline" | "arrow"; label: string; to?: Box | null };

const INK = "#38bdf8"; // one crisp cyan line, a soft shadow so it reads on any background
const GLOW = "drop-shadow(0 0 1.5px #000a) drop-shadow(0 1px 4px #0006)";

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

  // Where the ray from one box's centre towards another leaves the first box, plus a small gap.
  const edge = (b: Box, toward: { x: number; y: number }, gap = 8) => {
    const cx = b.x + b.w / 2, cy = b.y + b.h / 2;
    const dx = toward.x - cx, dy = toward.y - cy;
    const len = Math.hypot(dx, dy) || 1;
    const t = Math.min(b.w / 2 / (Math.abs(dx) / len || 1e-6), b.h / 2 / (Math.abs(dy) / len || 1e-6)) + gap;
    return { x: cx + (dx / len) * t, y: cy + (dy / len) * t };
  };

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
        const common = { stroke: INK, strokeWidth: 2.5, fill: "none", className: "mark", style: { animationDelay: delay } };
        // The arrow comes in from the upper left and stops at the item's edge.
        const cx = r.x + r.w / 2, cy = r.y + r.h / 2;
        const tail = { x: Math.max(20, cx - 170), y: Math.max(20, cy - 120) };
        // The label sits above its mark, or below it near the top edge, so it never covers what it points at.
        const above = r.y >= 44;
        const ly = above ? r.y - 30 : r.y + r.h + 10;
        return (
          <g key={i}>
            {m.kind === "box" && <rect x={r.x - 6} y={r.y - 4} width={r.w + 12} height={r.h + 8} rx={6} {...common} fill="#38bdf810" />}
            {m.kind === "circle" && <ellipse cx={cx} cy={cy} rx={r.w / 2 + 8} ry={r.h / 2 + 8} {...common} />}
            {m.kind === "underline" && <line x1={r.x} y1={r.y + r.h + 4} x2={r.x + r.w} y2={r.y + r.h + 4} {...common} strokeWidth={4} strokeLinecap="round" />}
            {m.kind === "arrow" && m.to && (() => {
              // A connector: from this item to that one, stopping at their edges.
              const t = px({ ...m, ...m.to });
              const a = edge(r, { x: t.x + t.w / 2, y: t.y + t.h / 2 }), b = edge(t, { x: cx, y: cy });
              return <line x1={a.x} y1={a.y} x2={b.x} y2={b.y} {...common} strokeWidth={4} strokeLinecap="round" markerEnd="url(#head)" />;
            })()}
            {m.kind === "arrow" && !m.to && <line x1={tail.x} y1={tail.y} x2={r.x + r.w * 0.1} y2={r.y + r.h * 0.1} {...common} strokeLinecap="round" markerEnd="url(#head)" />}
            {m.label && (
              <g className="mark" style={{ animationDelay: delay }}>
                <rect x={Math.min(r.x, size.w - 200)} y={ly} width={m.label.length * 7.4 + 18} height={22} rx={6} fill="#0a0a0ae6" />
                <text x={Math.min(r.x, size.w - 200) + 9} y={ly + 15} fill="#fff" fontSize="12.5" fontFamily="IBM Plex Sans, system-ui">
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
