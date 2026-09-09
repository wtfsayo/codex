use super::*;

#[test]
fn grouped_routes_cross_frame_borders_perpendicularly() {
    let scene = Scene {
        canvas: Canvas::new(24, 20),
        bounds: HashMap::from([
            (
                Item::Node(0),
                Placed {
                    x: 2,
                    y: 4,
                    w: 3,
                    h: 3,
                    cx: 3,
                    cy: 5,
                    rank: 0,
                },
            ),
            (
                Item::Group(0),
                Placed {
                    x: 5,
                    y: 5,
                    w: 16,
                    h: 12,
                    cx: 13,
                    cy: 11,
                    rank: 1,
                },
            ),
            (
                Item::Node(1),
                Placed {
                    x: 12,
                    y: 7,
                    w: 3,
                    h: 3,
                    cx: 13,
                    cy: 8,
                    rank: 0,
                },
            ),
        ]),
    };
    let path = route(&scene, Item::Node(0), Item::Node(1), &mut 100_000).unwrap();
    for step in path.windows(2) {
        let (x0, y0) = (step[0] % 24, step[0] / 24);
        let (x1, y1) = (step[1] % 24, step[1] / 24);
        assert!(
            !(y0 == y1 && [5, 16].contains(&y0) && (5..21).contains(&x0) && (5..21).contains(&x1)),
            "horizontal path follows frame border: {path:?}"
        );
        assert!(
            !(x0 == x1 && [5, 20].contains(&x0) && (5..17).contains(&y0) && (5..17).contains(&y1)),
            "vertical path follows frame border: {path:?}"
        );
    }
}

#[test]
fn grouped_edges_attach_to_children_in_all_directions() {
    let styles = MermaidStyles {
        border: Style::default(),
        node_text: Style::default(),
        edge: Style::default(),
        edge_label: Style::default(),
        title: Style::default(),
    };
    let mut output = String::new();
    for direction in ["LR", "TD", "RL", "BT"] {
        let source = format!(
            "flowchart {direction}\nsubgraph Clients\nA[Web]\nend\nsubgraph Backend\nsubgraph Services\nB[API]\nend\nC[Queue]\nend\nA --> B\nB -.-> C"
        );
        let art = super::super::render(&source, &styles, Some(120)).unwrap();
        assert!(
            !art.is_fallback,
            "{direction}: {}",
            art.plain_lines.join("\n")
        );
        output.push_str(&format!("{direction}\n{}\n\n", art.plain_lines.join("\n")));
    }
    for (name, source) in [
        (
            "self loop",
            "flowchart LR\nsubgraph Work\nA -->|retry| A\nend",
        ),
        (
            "parallel and bidirectional",
            "flowchart LR\nsubgraph Work\nA <--> B\nA --> B\nend",
        ),
        (
            "explicit group endpoints",
            "flowchart TD\nsubgraph Work\nA --> B\nend\nX --> Work\nWork --> Y",
        ),
    ] {
        let art = super::super::render(source, &styles, Some(120)).unwrap();
        assert!(!art.is_fallback, "{name}: {}", art.plain_lines.join("\n"));
        output.push_str(&format!("{name}\n{}\n\n", art.plain_lines.join("\n")));
    }
    insta::assert_snapshot!(output);
}

#[test]
fn grouped_edge_labels_are_not_silently_omitted() {
    let styles = MermaidStyles {
        border: Style::default(),
        node_text: Style::default(),
        edge: Style::default(),
        edge_label: Style::default(),
        title: Style::default(),
    };
    let source = "flowchart LR\nsubgraph One\nA\nend\nsubgraph Two\nB\nend\nA -->|a label that cannot fit within the supported width| B";
    let art = super::super::render(source, &styles, Some(120)).unwrap();
    assert!(art.is_fallback);
}
