use chrono::Duration;
use chrono::NaiveDate;
use chrono::NaiveDateTime;
use ratatui::style::Stylize;
use ratatui::text::Line;

#[cfg(test)]
#[path = "gantt_tests.rs"]
mod tests;

struct Task<'a> {
    label: String,
    section: &'a str,
    id: Option<&'a str>,
    start: Option<&'a str>,
    end: &'a str,
    milestone: bool,
}

struct Chart<'a> {
    title: Option<&'a str>,
    axis_format: &'a str,
    tasks: Vec<Task<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Schedule {
    start: NaiveDateTime,
    end: NaiveDateTime,
}

impl<'a> Chart<'a> {
    fn parse(source: &'a str) -> Option<Self> {
        let mut lines = source
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("%%"));
        if !lines.next()?.eq_ignore_ascii_case("gantt") {
            return None;
        }
        let mut chart = Self {
            title: None,
            axis_format: "%Y-%m-%d",
            tasks: Vec::new(),
        };
        let mut section = "";
        for line in lines {
            let directive = line.split_once(char::is_whitespace);
            match directive.map(|(key, value)| (key, value.trim())) {
                Some(("title", value)) if !value.is_empty() => chart.title = Some(value),
                Some(("section", value)) if !value.is_empty() => section = value,
                Some(("dateFormat", "YYYY-MM-DD")) => {}
                Some(("axisFormat", value)) if !value.is_empty() => {
                    // Accept only specifiers with identical date semantics in D3 and chrono.
                    let mut chars = value.chars();
                    while let Some(c) = chars.next() {
                        if c == '%'
                            && !matches!(
                                chars.next()?,
                                '%' | 'Y'
                                    | 'y'
                                    | 'm'
                                    | 'b'
                                    | 'B'
                                    | 'd'
                                    | 'e'
                                    | 'a'
                                    | 'A'
                                    | 'j'
                                    | 'H'
                                    | 'M'
                                    | 'S'
                            )
                        {
                            return None;
                        }
                    }
                    chart.axis_format = value;
                }
                Some(("todayMarker", "off")) => {}
                _ => {
                    let (label, metadata) = line.split_once(':')?;
                    let label = label.trim();
                    if label.is_empty() || chart.tasks.len() >= 128 {
                        return None;
                    }
                    let mut fields: Vec<_> = metadata.split(',').map(str::trim).collect();
                    let mut tags = Vec::new();
                    let mut milestone = false;
                    while let Some(&tag) = fields.first() {
                        match tag {
                            "milestone" => milestone = true,
                            "active" | "done" | "crit" => tags.push(tag),
                            _ => break,
                        }
                        fields.remove(0);
                    }
                    let (id, start, end) = match fields.as_slice() {
                        [end] => (None, None, *end),
                        [start, end] => (None, Some(*start), *end),
                        [id, start, end]
                            if !id.is_empty() && !id.chars().any(char::is_whitespace) =>
                        {
                            (Some(*id), Some(*start), *end)
                        }
                        _ => return None,
                    };
                    if id.is_some() && chart.tasks.iter().any(|task| task.id == id) {
                        return None;
                    }
                    chart.tasks.push(Task {
                        label: if tags.is_empty() {
                            label.to_owned()
                        } else {
                            format!("{label} [{}]", tags.join(", "))
                        },
                        section,
                        id,
                        start,
                        end,
                        milestone,
                    });
                }
            }
        }
        (!chart.tasks.is_empty()).then_some(chart)
    }

    fn schedule(&self) -> Option<Vec<Schedule>> {
        let mut resolved = vec![None; self.tasks.len()];
        let mut visiting = vec![false; self.tasks.len()];
        for index in 0..self.tasks.len() {
            self.resolve(index, &mut resolved, &mut visiting)?;
        }
        resolved.into_iter().collect()
    }

    fn resolve(
        &self,
        index: usize,
        resolved: &mut [Option<Schedule>],
        visiting: &mut [bool],
    ) -> Option<Schedule> {
        if let Some(schedule) = resolved[index] {
            return Some(schedule);
        }
        if visiting[index] {
            return None;
        }
        visiting[index] = true;
        let task = &self.tasks[index];
        let start = match task.start {
            Some(start) if start.starts_with("after ") => {
                let mut latest = None;
                for id in start[6..].split_whitespace() {
                    let dependency = self.tasks.iter().position(|task| task.id == Some(id))?;
                    let end = self.resolve(dependency, resolved, visiting)?.end;
                    latest = Some(latest.map_or(end, |value: NaiveDateTime| value.max(end)));
                }
                latest?
            }
            Some(start) => parse_date(start)?,
            None => self.resolve(index.checked_sub(1)?, resolved, visiting)?.end,
        };
        let end = if let Some(id) = task.end.strip_prefix("until ") {
            let dependency = self
                .tasks
                .iter()
                .position(|task| task.id == Some(id.trim()))?;
            self.resolve(dependency, resolved, visiting)?.start
        } else if let Some(date) = parse_date(task.end) {
            date
        } else {
            let (number, unit) = task
                .end
                .split_at(task.end.find(|c: char| !c.is_ascii_digit())?);
            let multiplier = match unit {
                "s" => 1,
                "m" => 60,
                "h" => 3600,
                "d" => 86400,
                "w" => 604800,
                _ => return None,
            };
            let seconds = number.parse::<i64>().ok()?.checked_mul(multiplier)?;
            start.checked_add_signed(Duration::try_seconds(seconds)?)?
        };
        if end < start {
            return None;
        }
        let schedule = Schedule { start, end };
        resolved[index] = Some(schedule);
        visiting[index] = false;
        Some(schedule)
    }
}

fn parse_date(value: &str) -> Option<NaiveDateTime> {
    if value.len() != 10
        || value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
    {
        return None;
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()?
        .and_hms_opt(/*hour*/ 0, /*min*/ 0, /*sec*/ 0)
}

pub(super) fn render(source: &str, width: usize) -> Option<Vec<Line<'static>>> {
    let chart = Chart::parse(source)?;
    let schedule = chart.schedule()?;
    let first = schedule.iter().map(|task| task.start).min()?;
    let last = schedule.iter().map(|task| task.end).max()?;
    let seconds = (last - first).num_seconds().max(1);
    // Scaling stays bounded even for pathological dates and dependency chains.
    if seconds > 86400 * 36525 {
        return None;
    }
    let label_width = chart
        .tasks
        .iter()
        .map(|task| crate::width::display_width(&task.label))
        .max()?;
    let columns = width.min(240).checked_sub(label_width + 3)?.min(80);
    let left_date = first.format(chart.axis_format).to_string();
    let right_date = last.format(chart.axis_format).to_string();
    let date_width =
        crate::width::display_width(&left_date) + crate::width::display_width(&right_date);
    if columns < date_width + 2 || columns < 12 {
        return None;
    }
    let position = |date: NaiveDateTime| {
        ((date - first).num_seconds() * (columns - 1) as i64 / seconds) as usize
    };
    let mut lines = Vec::new();
    if let Some(title) = chart.title {
        lines.extend(
            textwrap::wrap(title, width)
                .into_iter()
                .map(|line| Line::from(line.into_owned().bold())),
        );
    }
    let prefix = " ".repeat(label_width + 3);
    lines.push(Line::from(
        format!(
            "{prefix}{left_date}{}{right_date}",
            " ".repeat(columns - date_width)
        )
        .dim(),
    ));
    lines.push(Line::from(
        format!("{prefix}├{}┤", "─".repeat(columns - 2)).dim(),
    ));
    let mut previous_section = "";
    for (task, dates) in chart.tasks.iter().zip(schedule) {
        if task.section != previous_section {
            if crate::width::display_width(task.section) > width {
                return None;
            }
            lines.push(Line::from(task.section.to_owned().bold()));
            previous_section = task.section;
        }
        let mut bar = vec![' '; columns];
        let start = position(dates.start);
        let end = position(dates.end);
        if task.milestone {
            let middle = dates.start + (dates.end - dates.start) / 2;
            bar[position(middle)] = '◆';
        } else if start == end {
            bar[start] = '│';
        } else {
            bar[start..end].fill('━');
        }
        let padding = " ".repeat(label_width - crate::width::display_width(&task.label));
        let bar: String = bar.into_iter().collect();
        lines.push(Line::from(format!(
            "{}{padding} │ {}",
            task.label,
            bar.trim_end()
        )));
    }
    (lines.len() <= 256).then_some(lines)
}
