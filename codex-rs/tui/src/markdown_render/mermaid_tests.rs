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
        "stateDiagram-v2\nstate Outer {\n[*] --> Inner\n}\n[*] --> Outer\n```\n",
        "gantt\nA :a, after b, 1d\nB :b, after a, 1d\n```\n",
        "gantt\nA :a, after missing, 1d\n```\n",
        "gantt\nexcludes holidays\nA :2026-09-09, 1d\n```\n",
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
        ("gantt\nBuild :2026-09-09, 2d\n", 10),
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

#[test]
fn mermaid_grouped_architecture_renders_child_connections() {
    let source = "```mermaid\nflowchart LR\nsubgraph Clients\nWeb[Web app]\nMobile[Mobile app]\nend\nsubgraph Backend\nAPI[API server]\nQueue[Job queue]\nWorker[Worker]\nend\nsubgraph Storage\nDB[(Database)]\nFiles[(File storage)]\nend\nWeb --> API\nMobile --> API\nAPI --> DB\nAPI --> Queue\nQueue --> Worker\nWorker --> DB\nWorker --> Files\n```\n";
    let output = render(source, 160);
    assert!(
        !output.contains("flowchart LR"),
        "architecture stayed raw: {output}"
    );
    assert_eq!(
        output
            .chars()
            .filter(|c| matches!(c, '▶' | '◄' | '▲' | '▼'))
            .count(),
        7,
        "every connection needs its own arrowhead",
    );
    assert_snapshot!(output);
}

#[test]
fn mermaid_gantt_release_schedule_renders_timeline() {
    let source = "```mermaid\ngantt\ntitle Example release schedule\ndateFormat YYYY-MM-DD\naxisFormat %b %d\nsection Design\nRequirements :a1, 2026-09-09, 2d\nMockups :a2, after a1, 3d\nsection Development\nAPI :b1, after a1, 5d\nInterface :b2, after a2, 4d\nsection Release\nIntegration testing :c1, after b1 b2, 2d\nLaunch :milestone, after c1, 0d\n```\n";
    let output = render(source, 100);
    assert!(!output.contains("dateFormat"), "Gantt stayed raw: {output}");
    assert_snapshot!(output);
}

#[test]
fn mermaid_gantt_product_launch_excluding_weekends_renders_timeline() {
    let source = "```mermaid
gantt
    title Product launch – September to October 2026
    dateFormat YYYY-MM-DD
    axisFormat %b %d
    excludes weekends
    todayMarker off

    section Planning
    Project kickoff :milestone, done, kickoff, 2026-09-07, 0d
    Requirements :done, req, 2026-09-07, 3d
    Technical architecture :done, arch, after req, 3d
    Scope approved :done, milestone, scope, after arch, 0d

    section Design
    User flows :done, flows, after req, 3d
    Visual design :active, visual, after flows, 5d
    Interactive prototype :proto, after visual, 3d
    Usability testing :ux, after proto, 3d
    Design approved :milestone, design, after ux, 0d

    section Backend
    Database schema :done, schema, after arch, 2d
    Authentication :active, auth, after schema, 4d
    Core API :crit, api, after schema, 7d
    Background jobs :jobs, after api, 4d
    API integration tests :crit, api_tests, after auth api jobs, 3d

    section Frontend
    App shell and navigation :active, shell, after flows, 4d
    Shared components :components, after visual shell, 4d
    Main user flows :crit, frontend, after components api, 6d
    Accessibility review :a11y, after frontend, 3d
    UI refinements :polish, after a11y design, 3d

    section Infrastructure
    CI pipeline :done, ci, after arch, 2d
    Staging environment :staging, after ci, 3d
    Monitoring and alerts :monitor, after staging, 3d
    Production setup :prod, after monitor, 4d

    section Quality assurance
    End-to-end testing :crit, e2e, after api_tests polish staging, 5d
    Security review :security, after api_tests, 4d
    Performance testing :perf, after api_tests staging, 3d
    Fixes and regression :crit, fixes, after e2e security perf, 4d
    Release candidate :milestone, rc, after fixes, 0d

    section Launch
    Documentation :docs, after design, 6d
    Support training :training, after docs frontend, 3d
    Go / no-go review :crit, review, after rc prod training, 1d
    Public launch :milestone, launch, after review, 0d
    Launch monitoring :crit, watch, after launch, 3d
    Retrospective :retro, after watch, 1d
```
";
    let output = render(source, 100);
    assert!(
        !output.contains("excludes weekends"),
        "Gantt stayed raw: {output}"
    );
    assert_eq!(output.matches('◆').count(), 5);
    assert!(output.contains("Retrospective"));
    assert_snapshot!(output);
}
