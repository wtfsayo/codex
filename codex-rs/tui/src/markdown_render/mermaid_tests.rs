use super::render_markdown_text_with_width;
use insta::assert_snapshot;
use pretty_assertions::assert_eq;

fn render(source: &str, width: usize) -> String {
    render_markdown_text_with_width(source, Some(width)).to_string()
}

#[test]
fn mermaid_diagram_families() {
    let diagrams = [
        "flowchart TD\nA[Request] --> B{Allowed?}\nB -->|yes| C[Run]\nB -->|no| D[Reject]",
        "flowchart LR\nsubgraph Gateway\nA[Router] --> B[Agent]\nend\nGateway --> C[Model]",
        "sequenceDiagram\nparticipant User\nparticipant Agent\nUser->>Agent: Question\nAgent-->>User: Answer",
        "stateDiagram-v2\n[*] --> Idle\nIdle --> Running: start\nRunning --> Idle: done",
        "classDiagram\nclass Agent {\n+run()\n}\nAgent --> Session",
        "erDiagram\nAGENT ||--o{ SESSION : owns",
    ];
    let mut rendered = Vec::new();
    for diagram in diagrams {
        let markdown = format!("```mermaid\n{diagram}\n```\n");
        let output = render(&markdown, 100);
        assert_ne!(output.trim(), diagram, "diagram stayed raw: {diagram}");
        assert!(
            output
                .lines()
                .all(|line| crate::width::display_width(line) <= 100)
        );
        rendered.push(output);
    }
    assert_snapshot!(rendered.join("\n\n"));
}

#[test]
fn mermaid_fence_variants_and_container_indentation() {
    let body = "flowchart LR\nA[Build] --> B[Test]\n";
    let expected = render(&format!("```mermaid\n{body}```"), 80);
    for (open, close) in [
        ("```MeRmAiD theme=dark", "```"),
        ("~~~mermaid", "~~~~"),
        ("````mermaid,no_run", "`````"),
        ("```mermaid", "  ```  "),
    ] {
        assert_eq!(render(&format!("{open}\n{body}{close}\n"), 80), expected);
    }
    assert_eq!(
        render(
            "```mermaid\r\nflowchart LR\r\nA[Build] --> B[Test]\r\n```\r\n",
            80
        ),
        expected,
    );
    let quoted = render(
        "> ```mermaid\n> flowchart LR\n> A[Build] --> B[Test]\n> ```\n",
        82,
    );
    assert_eq!(
        quoted,
        expected
            .lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let listed = render(
        "- Pipeline\n\n  ```mermaid\n  flowchart LR\n  A[Build] --> B[Test]\n  ```\n",
        82,
    );
    assert_snapshot!(listed);
}

#[test]
fn mermaid_unclosed_or_unsupported_source_keeps_code_display() {
    let cases = [
        "flowchart LR\nA[Build] --> B[Test]\n",
        "flowchart LR\nA[Build] --> B[Test]\n``\n",
        "pie\n\"Part\" : 10\n```\n",
        "flowchart LR\nA[Unclosed --> B\n```\n",
        "flowchart LR\nA -->\n```\n",
        "flowchart LR\nA --> B unsupported syntax\n```\n",
        "flowchart LR\nsubgraph Group\nA --> B\n```\n",
        "flowchart LR\nsubgraph Group\ndirection BT\nA --> B\nend\n```\n",
        "flowchart LR\nsubgraph Group\nA\nB\nend\nX --> A\n```\n",
        "stateDiagram-v2\nstate Outer {\n[*] --> Inner\n}\n[*] --> Outer\n```\n",
    ];
    for body in cases {
        assert_eq!(
            render(&format!("```mermaid\n{body}"), 80),
            render(&format!("```text\n{body}"), 80),
            "fallback lost source for {body}",
        );
    }
    // A container ending implicitly is not a closing Mermaid fence.
    for markdown in [
        "> ```mermaid\n> flowchart LR\n> A --> B\n\nOutside\n",
        "- ```mermaid\n  flowchart LR\n  A --> B\n\nOutside\n",
    ] {
        assert_eq!(
            render(markdown, 80),
            render(&markdown.replace("mermaid", "text"), 80)
        );
    }
}

#[test]
fn mermaid_limits_fall_back_without_losing_source() {
    let large = format!("flowchart LR\nA --> B\n%% {}\n", "x".repeat(16 * 1024));
    let tall = format!("flowchart LR\nA --> B\n{}", "%% comment\n".repeat(512));
    let many_nodes = format!(
        "flowchart TD\n{}",
        (0..130)
            .map(|n| format!("N{n} --> N{}\n", n + 1))
            .collect::<String>()
    );
    for (body, width) in [
        (large.as_str(), 80),
        (tall.as_str(), 80),
        (many_nodes.as_str(), 80),
        ("flowchart LR\nA[Build] --> B[Test]\n", 1),
        ("flowchart LR\nA[\u{1b}[31m] --> B\n", 80),
    ] {
        assert_eq!(
            render(&format!("```mermaid\n{body}```\n"), width),
            render(&format!("```text\n{body}```\n"), width),
        );
    }
}

#[test]
fn mermaid_unicode_multiline_labels_and_surrounding_markdown() {
    let source = "Before.\n\n```mermaid\nflowchart TD\nA[\"入口<br/>Request\"] -->|\"next\"| B[\"Reply &amp; done\"]\n```\n\nAfter **diagram**.\n";
    assert_snapshot!(render(source, 80));
}
