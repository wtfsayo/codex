use super::*;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

type ScopeEdges = HashMap<Option<usize>, Vec<(Item, Item, usize)>>;

struct Scene {
    canvas: Canvas,
    bounds: HashMap<Item, Placed>,
}

pub(super) fn render(
    graph: &Graph,
    scope_edges: &ScopeEdges,
    direct_nodes: &HashMap<Option<usize>, Vec<usize>>,
    keep: &[bool],
    proxy: &HashMap<usize, usize>,
    max_width: Option<usize>,
) -> Result<Canvas, Oversize> {
    let mut scene = build(graph, /*scope*/ None, scope_edges, direct_nodes, keep)?;
    pad(&mut scene);
    if max_width.is_some_and(|width| scene.canvas.w > width) {
        return Err(Oversize::Width);
    }
    // Path search is bounded independently of the raster allocation limit.
    if scene.canvas.w.saturating_mul(scene.canvas.h) > 65_536 {
        return Err(Oversize::Cells);
    }
    scene.canvas.occupied.fill(false);
    for (item, rect) in &scene.bounds {
        if matches!(item, Item::Node(_)) {
            for y in rect.y..rect.y + rect.h {
                for x in rect.x..rect.x + rect.w {
                    let index = scene.canvas.idx(x, y);
                    scene.canvas.occupied[index] = true;
                }
            }
        }
    }
    let endpoint = |node| {
        proxy
            .get(&node)
            .map_or(Item::Node(node), |&group| Item::Group(group))
    };
    let mut budget = 1_000_000usize;
    for edge in &graph.edges {
        let from = endpoint(edge.from);
        let to = endpoint(edge.to);
        let path = route(&scene, from, to, &mut budget).ok_or(Oversize::Cells)?;
        paint(&mut scene.canvas, &path, edge)?;
    }
    scene.canvas.finalize_mask();
    Ok(scene.canvas)
}

fn build(
    graph: &Graph,
    scope: Option<usize>,
    scope_edges: &ScopeEdges,
    direct_nodes: &HashMap<Option<usize>, Vec<usize>>,
    keep: &[bool],
) -> Result<Scene, Oversize> {
    let mut items: Vec<Item> = direct_nodes
        .get(&scope)
        .into_iter()
        .flatten()
        .copied()
        .map(Item::Node)
        .collect();
    items.extend(
        (0..graph.groups.len())
            .filter(|&group| graph.groups[group].parent == scope && keep[group])
            .map(Item::Group),
    );
    if items.is_empty() {
        return Ok(Scene {
            canvas: Canvas::new(/*w*/ 1, /*h*/ 1),
            bounds: HashMap::new(),
        });
    }
    let mut nodes = Vec::new();
    let mut extras = Vec::new();
    let mut children = Vec::new();
    for item in &items {
        match *item {
            Item::Node(node) => {
                nodes.push(Node {
                    label: graph.nodes[node].label.clone(),
                    shape: graph.nodes[node].shape,
                });
                extras.push(NodeExtra::Plain);
                children.push(HashMap::new());
            }
            Item::Group(group) => {
                let mut child = build(graph, Some(group), scope_edges, direct_nodes, keep)?;
                pad(&mut child);
                nodes.push(Node {
                    label: graph.groups[group].label.clone(),
                    shape: Shape::Rect,
                });
                extras.push(NodeExtra::Frame(child.canvas));
                children.push(child.bounds);
            }
        }
    }
    let mut edges = Vec::new();
    for &(from, to, index) in scope_edges.get(&scope).into_iter().flatten() {
        let source = &graph.edges[index];
        let from = items
            .iter()
            .position(|item| *item == from)
            .ok_or(Oversize::Cells)?;
        let to = items
            .iter()
            .position(|item| *item == to)
            .ok_or(Oversize::Cells)?;
        edges.push(Edge {
            from,
            to,
            label: source.label.clone(),
            head_to: source.head_to,
            head_from: source.head_from,
            line: source.line,
        });
    }
    let synth = Graph {
        nodes,
        edges,
        index: HashMap::new(),
        groups: Vec::new(),
        node_group: Vec::new(),
        cur_group: None,
        over_cap: false,
        dir: graph.dir,
    };
    let (canvas, placed) = layout_canvas_placed(
        &synth,
        &extras,
        /*max_width*/ None,
        EdgeRendering::LayoutOnly,
    )?;
    let mut bounds = HashMap::new();
    for (index, item) in items.into_iter().enumerate() {
        let position = placed[index];
        if let NodeExtra::Frame(child) = &extras[index] {
            let ox = position.x + 1 + (position.w - 2 - child.w) / 2;
            let oy = position.y + 1 + (position.h - 2 - child.h) / 2;
            for (descendant, mut rect) in children[index].drain() {
                translate(&mut rect, ox, oy);
                bounds.insert(descendant, rect);
            }
        }
        bounds.insert(item, position);
    }
    Ok(Scene { canvas, bounds })
}

fn translate(rect: &mut Placed, x: usize, y: usize) {
    rect.x += x;
    rect.cx += x;
    rect.y += y;
    rect.cy += y;
}

fn pad(scene: &mut Scene) {
    let mut canvas = Canvas::new(scene.canvas.w + 4, scene.canvas.h + 4);
    canvas.blit(&scene.canvas, /*ox*/ 2, /*oy*/ 2);
    for rect in scene.bounds.values_mut() {
        translate(rect, /*x*/ 2, /*y*/ 2);
    }
    scene.canvas = canvas;
}

fn contains(rect: &Placed, x: usize, y: usize) -> bool {
    x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h
}

fn ports(rect: &Placed, width: usize) -> [(usize, usize); 4] {
    let right = rect.cy * width + rect.x + rect.w;
    let bottom = (rect.y + rect.h) * width + rect.cx;
    let left = rect.cy * width + rect.x - 1;
    let top = (rect.y - 1) * width + rect.cx;
    [
        (right + 1, right),
        (bottom + width, bottom),
        (left - 1, left),
        (top - width, top),
    ]
}

fn route(scene: &Scene, from: Item, to: Item, budget: &mut usize) -> Option<Vec<usize>> {
    let canvas = &scene.canvas;
    let source = scene.bounds.get(&from)?;
    let target = scene.bounds.get(&to)?;
    let mut blocked = vec![false; canvas.w * canvas.h];
    let mut border = vec![0u8; blocked.len()];
    for (&item, rect) in &scene.bounds {
        let ancestor = matches!(item, Item::Group(_))
            && item != from
            && item != to
            && (contains(rect, source.cx, source.cy) || contains(rect, target.cx, target.cy));
        if !ancestor {
            for y in rect.y..rect.y + rect.h {
                for x in rect.x..rect.x + rect.w {
                    blocked[canvas.idx(x, y)] = true;
                }
            }
        } else {
            for y in rect.y..rect.y + rect.h {
                for x in rect.x..rect.x + rect.w {
                    let horizontal = y == rect.y || y + 1 == rect.y + rect.h;
                    let vertical = x == rect.x || x + 1 == rect.x + rect.w;
                    let index = canvas.idx(x, y);
                    blocked[index] |= horizontal && vertical;
                    border[index] |= if horizontal { L | R } else { 0 };
                    border[index] |= if vertical { U | D } else { 0 };
                }
            }
        }
    }
    for (index, cell) in blocked.iter_mut().enumerate() {
        *cell |= matches!(canvas.cls[index], Cls::Text | Cls::EdgeLabel) || canvas.occupied[index];
    }
    let mut starts = ports(source, canvas.w).to_vec();
    let mut ends = ports(target, canvas.w).to_vec();
    if from == to {
        starts.truncate(1);
        ends = vec![ends[1]];
    }
    let valid_port = |&(far, near): &(usize, usize)| {
        let movement = if far / canvas.w == near / canvas.w {
            L | R
        } else {
            U | D
        };
        far < blocked.len()
            && !blocked[far]
            && !blocked[near]
            && (border[far] | border[near]) & movement == 0
    };
    starts.retain(valid_port);
    ends.retain(valid_port);
    for &(start_far, start_near) in &starts {
        if let Some(&(_, end_near)) = ends
            .iter()
            .find(|&&(far, near)| far == start_near && near == start_far)
        {
            return Some(vec![start_near, end_near]);
        }
    }
    for &(_, near) in starts.iter().chain(&ends) {
        blocked[near] = true;
    }
    let mut distance = vec![usize::MAX; blocked.len()];
    let mut previous = vec![usize::MAX; blocked.len()];
    let mut queue = BinaryHeap::new();
    for &(start, _) in &starts {
        if !blocked[start] {
            distance[start] = 0;
            queue.push(Reverse((0usize, start)));
        }
    }
    while let Some(Reverse((cost, current))) = queue.pop() {
        *budget = budget.checked_sub(1)?;
        if cost != distance[current] {
            continue;
        }
        if let Some(&(_, end_port)) = ends.iter().find(|&&(far, _)| far == current) {
            let mut path = vec![current];
            while previous[*path.last()?] != usize::MAX {
                path.push(previous[*path.last()?]);
            }
            path.reverse();
            let start_port = starts.iter().find(|&&(far, _)| far == path[0])?.1;
            path.insert(0, start_port);
            path.push(end_port);
            return Some(path);
        }
        let x = current % canvas.w;
        let y = current / canvas.w;
        let neighbors = [
            x.checked_sub(1).map(|x| canvas.idx(x, y)),
            (x + 1 < canvas.w).then(|| canvas.idx(x + 1, y)),
            y.checked_sub(1).map(|y| canvas.idx(x, y)),
            (y + 1 < canvas.h).then(|| canvas.idx(x, y + 1)),
        ];
        for next in neighbors.into_iter().flatten() {
            let movement = if next / canvas.w == y { L | R } else { U | D };
            if blocked[next] || (border[current] | border[next]) & movement != 0 {
                continue;
            }
            let turn = previous[current] != usize::MAX
                && (previous[current] / canvas.w == y) != (next / canvas.w == y);
            let cost = cost + 1 + usize::from(turn) * 3 + usize::from(canvas.mask[next] != 0) * 4;
            if cost < distance[next] {
                distance[next] = cost;
                previous[next] = current;
                queue.push(Reverse((cost, next)));
            }
        }
    }
    None
}

fn direction(from: usize, to: usize, width: usize) -> (u8, char) {
    if from / width == to / width {
        if from < to { (R, '▶') } else { (L, '◄') }
    } else if from < to {
        (D, '▼')
    } else {
        (U, '▲')
    }
}

fn paint(canvas: &mut Canvas, path: &[usize], edge: &Edge) -> Result<(), Oversize> {
    canvas.cur_style = match edge.line {
        LineKind::Solid => STY_SOLID,
        LineKind::Dotted => STY_DOT,
        LineKind::Thick => STY_THICK,
    };
    for (position, &index) in path.iter().enumerate() {
        // A path may cross an ancestor's border, but never its title or a node.
        canvas.mask[index] |= match canvas.ch[index] {
            '─' => L | R,
            '│' => U | D,
            '┌' => D | R,
            '┐' => D | L,
            '└' => U | R,
            '┘' => U | L,
            _ => 0,
        };
        canvas.ch[index] = ' ';
        let mut bits = 0;
        if position > 0 {
            bits |= direction(index, path[position - 1], canvas.w).0;
        }
        if position + 1 < path.len() {
            bits |= direction(index, path[position + 1], canvas.w).0;
        }
        canvas.add_bits(index % canvas.w, index / canvas.w, bits);
    }
    for (index, neighbor, head) in [
        (path[0], path[1], edge.head_from),
        (path[path.len() - 1], path[path.len() - 2], edge.head_to),
    ] {
        if head != Head::None {
            let arrow = direction(neighbor, index, canvas.w).1;
            canvas.ch[index] = head_glyph(head, arrow);
            canvas.occupied[index] = true;
        }
    }
    if let Some(label) = &edge.label {
        if label.width() > MAX_LABEL {
            return Err(Oversize::Cells);
        }
        let text = label;
        let mut placed = false;
        for &index in path.iter().rev() {
            let x = index % canvas.w;
            let y = index / canvas.w;
            for (start, row) in [(x + 1, y), (x, y.saturating_sub(1))] {
                if start + text.width() > canvas.w
                    || !(start..start + text.width()).all(|column| {
                        let cell = canvas.idx(column, row);
                        canvas.ch[cell] == ' ' && canvas.mask[cell] == 0 && !canvas.occupied[cell]
                    })
                {
                    continue;
                }
                draw_seq_text(canvas, text, start, row, Cls::EdgeLabel);
                placed = true;
                break;
            }
            if placed {
                break;
            }
        }
        if !placed {
            return Err(Oversize::Cells);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "grouped_tests.rs"]
mod tests;
