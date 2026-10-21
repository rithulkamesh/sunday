import type { SidebarsConfig } from "@docusaurus/plugin-content-docs";

const sidebars: SidebarsConfig = {
  docs: [
    { type: "category", label: "Use Sunday", collapsed: false, items: ["getting-started", "features", "providers", "roadmap"] },
    { type: "category", label: "Build on Sunday", collapsed: false, items: ["architecture", "repo-map", "router", "testing", "evals", "agents", "plugins"] },
  ],
};

export default sidebars;
