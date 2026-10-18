import type { SidebarsConfig } from "@docusaurus/plugin-content-docs";

const sidebars: SidebarsConfig = {
  docs: [
    { type: "category", label: "Use Sunday", collapsed: false, items: ["getting-started", "features", "providers"] },
    { type: "category", label: "Build on Sunday", collapsed: false, items: ["repo-map", "agents", "evals", "plugins"] },
  ],
};

export default sidebars;
