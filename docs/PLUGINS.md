---
id: plugins
slug: /plugins
title: Plugins
---

# Plugins

> **Planned.** Nothing here ships yet. This page marks where the guide will live and tracks the work.

Sunday is meant to be extended without forking it. The pieces, each tracked as an issue:

| Piece | What it will let you do | Issue |
|---|---|---|
| Tool SDK | Add an action the agent can use, with its own safety check | [#40](https://github.com/rithulkamesh/sunday/issues/40) |
| MCP client | Load tools from any MCP server | [#7](https://github.com/rithulkamesh/sunday/issues/7) |
| Hooks and events | React to an utterance, an action or a result | [#43](https://github.com/rithulkamesh/sunday/issues/43) |
| Skill packs | Share, import and sign skills | [#42](https://github.com/rithulkamesh/sunday/issues/42) |
| Pluggable router and planner | Replace the decision logic | [#41](https://github.com/rithulkamesh/sunday/issues/41) |

## Ground rules for anything that extends the agent

- A new action goes through the same checks as the built-in ones. It can't skip approval.
- What a plugin reads (web, screen, files, memory) is data. It never becomes a command by itself.
- A plugin can't edit the permission layer.
- Any network call a plugin makes is off until the user turns it on.

Want to help? Pick one of the issues above and read [Working with agents](AGENTS.md).
