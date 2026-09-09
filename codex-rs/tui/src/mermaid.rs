//! Bounded terminal diagrams. The transcript always retains the original Mermaid source.

use ratatui::style::Style;
use ratatui::text::Line;

mod gantt;
mod renderer;

const MAX_SOURCE_BYTES: usize = 16 * 1024;
const MAX_SOURCE_LINES: usize = 512;
const MAX_RENDERED_LINES: usize = 256;
const MAX_COLUMNS: usize = 240;

pub(crate) fn render(source: &str, width: Option<usize>) -> Option<Vec<Line<'static>>> {
    // Bounds apply before parsing, including statements which never become graph nodes.
    if source.len() > MAX_SOURCE_BYTES
        || source.lines().count() > MAX_SOURCE_LINES
        || source
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return None;
    }
    let width = width.unwrap_or(120).min(MAX_COLUMNS);
    let lines = if source
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("%%"))
        .is_some_and(|line| line.eq_ignore_ascii_case("gantt"))
    {
        gantt::render(source, width)?
    } else {
        let styles = renderer::MermaidStyles {
            border: Style::new().dim(),
            node_text: Style::default(),
            edge: Style::default(),
            edge_label: Style::default(),
            title: Style::new().bold(),
        };
        let art = renderer::render(source, &styles, Some(width))?;
        if art.is_fallback {
            return None;
        }
        let start = art
            .plain_lines
            .iter()
            .position(|line| !line.trim().is_empty())?;
        let end = art
            .plain_lines
            .iter()
            .rposition(|line| !line.trim().is_empty())?;
        art.styled_lines
            .into_iter()
            .skip(start)
            .take(end - start + 1)
            .collect()
    };
    // Keep ordinary source display when layout cannot fit; never clip diagram edges.
    if lines.len() > MAX_RENDERED_LINES
        || lines.iter().any(|line| {
            let text = line.to_string();
            crate::width::display_width(&text) > width || text.chars().any(char::is_control)
        })
    {
        return None;
    }
    Some(lines)
}
