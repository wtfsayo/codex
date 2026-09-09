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
fn mermaid_stream_keeps_source_mutable_through_close_and_following_prose() {
    let cwd = std::env::temp_dir();
    for source in [
        "Before.\n\n```mermaid,no_run\ngraph TD\nA[Input] --> B[Output]\n```\n\nAfter.\n",
        "Before.\n\n> ~~~~MeRmAiD title=Flow\n> graph TD\n> A[Input] --> B[Output]\n> ~~~~\n\nAfter.\n",
        "Before.\n\n- ````mermaid\n  graph TD\n  A[Input] --> B[Output]\n  ````\n\nAfter.\n",
        "Before.\n\n1. Diagram\n\n   ```mermaid\n   graph TD\n   A[Input] --> B[Output]\n   ```\n\nAfter.\n",
        "Before.\n\n````markdown\n```mermaid\ngraph TD\nA[Input] --> B[Output]\n```\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n````\n\nAfter.\n",
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
                plain_lines(&render_source(
                    &committed,
                    /*width*/ Some(80),
                    &cwd,
                    HistoryRenderMode::Rich,
                    /*inline_visualization_context*/ None,
                )),
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
    let mut controller =
        StreamController::new(/*width*/ Some(80), &cwd, HistoryRenderMode::Rich);
    let mut emitted = Vec::new();
    let open = "```mermaid\ngraph LR\nA[Incoming message] --> B[Gateway]\n";
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
    controller.push("```\n");
    let source = format!("{open}```\n");
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
