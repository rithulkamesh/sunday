import type { Config } from "@docusaurus/types";
import type * as Preset from "@docusaurus/preset-classic";

const config: Config = {
  title: "Sunday",
  tagline: "Can any model go AGI, given the right harness?",
  favicon: "img/favicon.svg",
  url: "https://rithulkamesh.github.io",
  baseUrl: "/sunday/",
  organizationName: "rithulkamesh",
  projectName: "sunday",
  onBrokenLinks: "throw",
  // The repo's docs are plain CommonMark, not MDX.
  markdown: { format: "detect", hooks: { onBrokenMarkdownLinks: "throw" } },
  presets: [
    [
      "classic",
      {
        docs: { path: "../docs", routeBasePath: "docs", sidebarPath: "./sidebars.ts", editUrl: "https://github.com/rithulkamesh/sunday/edit/main/docs/" },
        blog: false,
        theme: { customCss: "./src/css/custom.css" },
      } satisfies Preset.Options,
    ],
  ],
  themeConfig: {
    colorMode: { defaultMode: "dark", disableSwitch: true, respectPrefersColorScheme: false },
    navbar: {
      title: "Sunday",
      logo: { alt: "", src: "img/mark.svg" },
      items: [
        { to: "/docs/", label: "Docs", position: "left" },
        { to: "/docs/providers", label: "Providers", position: "left" },
        { to: "/docs/evals", label: "Evals", position: "left" },
        { href: "https://github.com/rithulkamesh/sunday/issues/31", label: "Roadmap", position: "left" },
        { href: "https://github.com/rithulkamesh/sunday", label: "GitHub", position: "right" },
      ],
    },
    footer: {
      style: "dark",
      links: [
        { title: "Use it", items: [{ label: "Get started", to: "/docs/" }, { label: "Features", to: "/docs/features" }, { label: "Providers", to: "/docs/providers" }] },
        { title: "Build on it", items: [{ label: "Repo map", to: "/docs/repo-map" }, { label: "Working with agents", to: "/docs/agents" }, { label: "Plugins", to: "/docs/plugins" }] },
        { title: "Follow along", items: [{ label: "v1 tracker", href: "https://github.com/rithulkamesh/sunday/issues/31" }, { label: "GitHub", href: "https://github.com/rithulkamesh/sunday" }] },
      ],
      copyright: "GPL-3.0-or-later. An open experiment.",
    },
    prism: { additionalLanguages: ["bash", "rust", "json", "toml"] },
  } satisfies Preset.ThemeConfig,
};

export default config;
