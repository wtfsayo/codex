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
- Route grouped connections to their original node or group endpoints, including
  nested subgraphs. Group projections determine placement only. Connections avoid
  node boxes and labels and cross only the endpoint groups' ancestor frames.
- Replace image-viewer guidance with terminal-width guidance.
- Add regression coverage for strict parsing and the adapter's fallback outcome.

The separate Gantt renderer supports sections, ISO dates, explicit end dates,
integer second/minute/hour/day/week durations, sequential task starts, `after`
dependencies, `until` end references, task statuses, and milestones. A task with
multiple prerequisites starts at their latest end. Milestones appear at the
midpoint of their declared interval. Missing dependencies and cycles retain the
source. `excludes weekends` follows Mermaid's Saturday/Sunday scheduling rules:
dependency endpoints advance across excluded days, while bars omit trailing
excluded days. Explicit starts and fixed end dates stay as declared. Other
exclusions, custom input date formats, and calendar-month durations are not
supported.

This is a terminal-oriented subset of Mermaid. Supported diagram families are
flowcharts, sequence diagrams, state diagrams, class diagrams, ER diagrams, and
Gantt charts.
Styling directives and interactive links are not applied. Composite and concurrent
state diagrams and state notes use source fallback. Sequence activation bars are
not drawn. Grouped routing has a bounded search budget and preserves the source
if a connection or its label cannot fit without overwriting nodes or arrowheads.
Labels wrap within bounded boxes and may be shortened with an ellipsis. Layout
has node, edge, group, and canvas limits; the parent adapter bounds source size
and preserves the original Markdown when a diagram cannot be displayed.
