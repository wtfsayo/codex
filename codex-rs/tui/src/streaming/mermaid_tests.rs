use super::*;
use pretty_assertions::assert_eq;

fn plain_lines(lines: &[HyperlinkLine]) -> Vec<String> {
    lines.iter().map(|line| line.line.to_string()).collect()
}

fn drain(controller: &mut StreamController, emitted: &mut Vec<String>) {
    let (cell, _) = controller.on_commit_tick_batch(usize::MAX);
    if let Some(cell) = cell {
        emitted.extend(
            cell.transcript_lines(u16::MAX)
                .iter()
                .map(|line| line.to_string().chars().skip(2).collect()),
        );
    }
}

#[test]
fn mermaid_stream_progressively_renders_before_closing_fence() {
    let cwd = std::env::temp_dir();
    let mut snapshots = Vec::new();
    for (opening, first, next, closing) in [
        (
            "```mermaid\nflowchart LR\n",
            "A[入口] --> B[Gateway]\n",
            "B --> C[Reply]\n",
            "```\n",
        ),
        (
            "> ~~~~MeRmAiD title=Flow\n> flowchart LR\n",
            "> A[入口] --> B[Gateway]\n",
            "> B --> C[Reply]\n",
            "> ~~~~\n",
        ),
        (
            "- ````mermaid\n  flowchart LR\n",
            "  A[入口] --> B[Gateway]\n",
            "  B --> C[Reply]\n",
            "  ````\n",
        ),
        (
            "```mermaid\ngantt\naxisFormat %b %d\nexcludes weekends\n",
            "Build :a, 2026-09-11, 2d\n",
            "Ship :milestone, after a, 0d\n",
            "```\n",
        ),
        (
            "```mermaid\nsequenceDiagram\n",
            "User->>API: Request\n",
            "API-->>User: Reply\n",
            "```\n",
        ),
        (
            "```mermaid\nflowchart LR\n",
            "subgraph Clients\nA[Web]\nend\n",
            "subgraph Backend\nB[API]\nend\nA --> B\n",
            "```\n",
        ),
    ] {
        let mut controller =
            StreamController::new(/*width*/ Some(100), &cwd, HistoryRenderMode::Rich);
        let mut emitted = Vec::new();
        let mut source = format!("{opening}{first}");
        controller.push(&source);
        drain(&mut controller, &mut emitted);
        assert!(emitted.is_empty());
        let expected = render_source(
            &format!("{source}{closing}"),
            Some(100),
            &cwd,
            HistoryRenderMode::Rich,
            /*inline_visualization_context*/ None,
        );
        assert_eq!(
            controller.current_tail_lines(),
            expected,
            "first statement must render before the closing fence"
        );
        snapshots.push(plain_lines(&expected).join("\n"));
        let before = controller.current_tail_lines();
        for line in next.split_inclusive('\n') {
            let before_line = controller.current_tail_lines();
            for c in line.chars() {
                controller.push(&c.to_string());
                if c != '\n' {
                    assert_eq!(controller.current_tail_lines(), before_line);
                }
            }
        }
        source.push_str(next);
        drain(&mut controller, &mut emitted);
        assert!(emitted.is_empty());
        let expected = render_source(
            &format!("{source}{closing}"),
            Some(100),
            &cwd,
            HistoryRenderMode::Rich,
            /*inline_visualization_context*/ None,
        );
        assert_eq!(
            controller.current_tail_lines(),
            expected,
            "new statements must update the preview"
        );
        assert_ne!(controller.current_tail_lines(), before);
        snapshots.push(plain_lines(&expected).join("\n"));
        let (cell, original) = controller.finalize();
        assert_eq!(original.as_deref(), Some(source.as_str()));
        assert_eq!(
            cell.unwrap()
                .transcript_lines(u16::MAX)
                .iter()
                .map(|line| line.to_string().chars().skip(2).collect::<String>())
                .collect::<Vec<_>>(),
            plain_lines(&render_source(
                &source,
                Some(100),
                &cwd,
                HistoryRenderMode::Rich,
                /*inline_visualization_context*/ None
            ))
            .into_iter()
            .map(|line| line.trim_end().to_owned())
            .collect::<Vec<_>>()
        );
    }
    let snapshot = snapshots.join("\n\n");
    let snapshot = snapshot
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(snapshot);
}

#[test]
fn mermaid_stream_keeps_source_mutable_through_close_and_following_prose() {
    let cwd = std::env::temp_dir();
    for source in [
        "Before.\n\n```mermaid,no_run\ngraph TD\nA[Input] --> B[Output]\n```\n\nAfter.\n",
        "Before.\n\n> ~~~~MeRmAiD title=Flow\n> graph TD\n> A[Input] --> B[Output]\n> ~~~~\n\nAfter.\n",
        "Before.\n\n- ````mermaid\n  graph TD\n  A[Input] --> B[Output]\n  ````\n\nAfter.\n",
        "Before.\n\n1. Diagram\n\n   ```mermaid\n   graph TD\n   A[Input] --> B[Output]\n   ```\n\nAfter.\n",
        "Before.\n\n````markdown\n```mermaid\ngraph TD\nA[Input] --> B[Output]\n```\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n````\n\nAfter.\n",
        "Before.\n\n```mermaid\nflowchart LR\nsubgraph Clients\nA[Input]\nend\nsubgraph Backend\nB[Output]\nend\nA --> B\n```\n\nAfter.\n",
        "Before.\n\n```mermaid\ngantt\naxisFormat %b %d\nexcludes weekends\nBuild :a, 2026-09-11, 2d\nShip :milestone, after a, 0d\n```\n\nAfter.\n",
    ] {
        let mut controller =
            StreamController::new(/*width*/ Some(80), &cwd, HistoryRenderMode::Rich);
        let mut emitted = Vec::new();
        let mut committed = String::new();
        for delta in source.split_inclusive('\n') {
            // Exercise provider deltas splitting the language, code, and closing fence.
            for fragment in delta.as_bytes().chunks(3) {
                controller.push(std::str::from_utf8(fragment).unwrap());
                drain(&mut controller, &mut emitted);
            }
            committed.push_str(delta);
            assert!(
                emitted.iter().all(|line| !line.contains("graph TD")),
                "diagram source escaped into scrollback: {emitted:?}",
            );
            let mut visible = emitted.clone();
            visible.extend(plain_lines(&controller.current_tail_lines()));
            assert_eq!(
                visible,
                plain_lines(
                    &crate::markdown::render_streaming_markdown_agent_with_links_and_cwd(
                        &committed,
                        /*width*/ Some(80),
                        Some(&cwd),
                    )
                    .lines
                ),
                "stream diverged after {committed:?}",
            );
        }
        let (cell, original) = controller.finalize();
        assert_eq!(original.as_deref(), Some(source));
        emitted.extend(
            cell.unwrap()
                .transcript_lines(u16::MAX)
                .iter()
                .map(|line| line.to_string().chars().skip(2).collect()),
        );
        assert_eq!(
            emitted,
            plain_lines(&render_source(
                source,
                /*width*/ Some(80),
                &cwd,
                HistoryRenderMode::Rich,
                /*inline_visualization_context*/ None,
            ))
            .into_iter()
            // History-cell wrapping trims trailing whitespace on otherwise blank rows.
            .map(|line| line.trim_end().to_owned())
            .collect::<Vec<_>>(),
        );
    }
}

#[test]
fn mermaid_stream_resize_and_raw_toggle_preserve_diagram_and_original_source() {
    let cwd = std::env::temp_dir();
    for open in [
        "```mermaid\ngraph LR\nA[Incoming message] --> B[Gateway]\n",
        "```mermaid\nflowchart LR\nsubgraph Clients\nA[Input]\nend\nsubgraph Backend\nB[Output]\nend\nA --> B\n",
        "```mermaid\ngantt\naxisFormat %b %d\nexcludes weekends\nBuild :a, 2026-09-11, 2d\nShip :milestone, after a, 0d\n",
    ] {
        let mut controller =
            StreamController::new(/*width*/ Some(80), &cwd, HistoryRenderMode::Rich);
        let mut emitted = Vec::new();
        controller.push(open);
        drain(&mut controller, &mut emitted);
        assert_eq!(emitted, Vec::<String>::new());
        controller.set_render_mode(HistoryRenderMode::Raw);
        drain(&mut controller, &mut emitted);
        assert_eq!(emitted, Vec::<String>::new());
        assert_eq!(
            plain_lines(&controller.current_tail_lines()),
            open.lines().map(str::to_owned).collect::<Vec<_>>(),
        );
        let source = format!("{open}```\n");
        for width in [30, 120] {
            controller.set_width(Some(width));
            for mode in [HistoryRenderMode::Rich, HistoryRenderMode::Raw] {
                controller.set_render_mode(mode);
                drain(&mut controller, &mut emitted);
                assert!(emitted.is_empty());
                let expected_source = if mode == HistoryRenderMode::Rich {
                    &source
                } else {
                    open
                };
                assert_eq!(
                    controller.current_tail_lines(),
                    render_source(
                        expected_source,
                        Some(width),
                        &cwd,
                        mode,
                        /*inline_visualization_context*/ None
                    ),
                    "open preview must survive resize and raw toggle"
                );
            }
        }
        controller.push("```\n");
        for width in [30, 120] {
            controller.set_width(Some(width));
            for mode in [HistoryRenderMode::Rich, HistoryRenderMode::Raw] {
                controller.set_render_mode(mode);
                drain(&mut controller, &mut emitted);
                assert_eq!(emitted, Vec::<String>::new());
                assert_eq!(
                    controller.current_tail_lines(),
                    render_source(
                        &source,
                        Some(width),
                        &cwd,
                        mode,
                        /*inline_visualization_context*/ None,
                    ),
                );
            }
        }
        controller.set_render_mode(HistoryRenderMode::Rich);
        let (cell, original) = controller.finalize();
        assert!(cell.is_some());
        assert_eq!(original, Some(source));
    }
}

#[test]
fn closed_markdown_wrapper_without_diagram_resumes_history_emission() {
    let cwd = std::env::temp_dir();
    let mut controller =
        StreamController::new(/*width*/ Some(80), &cwd, HistoryRenderMode::Rich);
    let source = "Before.\n\n````markdown\n```sh\nprintf 'hello'\n```\n````\n\n```json\n{\"done\": true}\n```\n";
    let mut emitted = Vec::new();
    for delta in source.split_inclusive('\n') {
        controller.push(delta);
        drain(&mut controller, &mut emitted);
    }
    assert_eq!(
        emitted,
        plain_lines(&render_source(
            source,
            /*width*/ Some(80),
            &cwd,
            HistoryRenderMode::Rich,
            /*inline_visualization_context*/ None,
        )),
    );
    let (remaining, original) = controller.finalize();
    assert!(remaining.is_none());
    assert_eq!(original.as_deref(), Some(source));
}
