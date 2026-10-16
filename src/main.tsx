import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./index.css";
import { Hud } from "./Hud";
import { Overlay } from "./Overlay";
import { Workspace } from "./Workspace";

const label = getCurrentWindow().label;
// The HUD and the screen overlay are transparent; the workspace paints its own background.
if (label === "main") document.body.classList.add("bg-canvas");

createRoot(document.getElementById("root")!).render(label === "hud" ? <Hud /> : label === "overlay" ? <Overlay /> : <Workspace />);
