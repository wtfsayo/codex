# Native Mermaid renderer

Completed `mermaid` code fences render automatically as terminal diagrams.
Press Alt+R to toggle the original Markdown. Unsupported or oversized diagrams
keep the ordinary code-block presentation.

`renderer.rs` includes an adapted Apache-2.0 terminal renderer:

- Copyright 2023-2026 SpaceXAI.
- Upstream commit: `75810042ca2762aa0b0fa17864f3f68823ccbea5`.
- Verified upstream Git blob: `73691433ac387d6d8fd1b023fbe976bcded70d3f`.
- License: Apache License 2.0, included in the repository root `LICENSE`.

The native renderer draws Unicode text with Ratatui and unicode-width. It does
not execute Mermaid JavaScript, start a browser, write image files, or require
terminal image protocols.

The imported renderer and its existing tests remain together to make comparison
with the upstream implementation straightforward. This accounts for the module's
size. New Codex integration logic belongs in the parent adapter.

Codex modifications:

- Expose whether rendering used the raw-source fallback so the adapter can retain
  Codex's existing code-block presentation.
- Reject incomplete flowchart statements, shapes, subgraphs, unknown directions,
  and scoped direction overrides instead of rendering a partial interpretation.
- Reject incomplete state, class, ER, and sequence blocks; unknown diagram header
  suffixes; class namespaces; and malformed sequence participant/note scopes.
- Preserve source for composite or concurrent state regions and state notes,
  whose scopes and content the native renderer cannot represent.
- Preserve source for edges crossing a subgraph boundary to a member node.
  Explicit edges between peer group IDs and edges wholly inside a group render.
- Replace image-viewer guidance with terminal-width guidance.
- Add regression coverage for strict parsing and the adapter's fallback outcome.

This is a terminal-oriented subset of Mermaid. Supported diagram families are
flowcharts, sequence diagrams, state diagrams, class diagrams, and ER diagrams.
Styling directives and interactive links are not applied. Composite and concurrent
state diagrams and state notes use source fallback, as do edges that cross a
subgraph boundary to a member node. Sequence activation bars are not drawn.
Labels wrap within bounded boxes and may be shortened with an ellipsis. Layout
has node, edge, group, and canvas limits; the parent adapter bounds source size
and preserves the original Markdown when a diagram cannot be displayed.
