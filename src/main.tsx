import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./index.css";
import { Hud } from "./Hud";
import { Workspace } from "./Workspace";

const hud = getCurrentWindow().label === "hud";
// The HUD is a transparent overlay; the workspace paints its own background.
if (!hud) document.body.classList.add("bg-canvas");

createRoot(document.getElementById("root")!).render(hud ? <Hud /> : <Workspace />);
