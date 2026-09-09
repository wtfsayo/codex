use super::Chart;
use super::render;
use pretty_assertions::assert_eq;

const RELEASE_SCHEDULE: &str = "gantt
    title Example release schedule
    dateFormat YYYY-MM-DD
    axisFormat %b %d
    section Design
    Requirements :a1, 2026-09-09, 2d
    Mockups :a2, after a1, 3d
    section Development
    API :b1, after a1, 5d
    Interface :b2, after a2, 4d
    section Release
    Integration testing :c1, after b1 b2, 2d
    Launch :milestone, after c1, 0d";

#[test]
fn gantt_release_schedule_uses_latest_dependency_end() {
    let chart = Chart::parse(RELEASE_SCHEDULE).expect("valid release schedule");
    let dates: Vec<_> = chart
        .schedule()
        .expect("resolved dates")
        .into_iter()
        .map(|task| (task.start.to_string(), task.end.to_string()))
        .collect();
    assert_eq!(
        dates,
        [
            ("2026-09-09 00:00:00", "2026-09-11 00:00:00"),
            ("2026-09-11 00:00:00", "2026-09-14 00:00:00"),
            ("2026-09-11 00:00:00", "2026-09-16 00:00:00"),
            ("2026-09-14 00:00:00", "2026-09-18 00:00:00"),
            ("2026-09-18 00:00:00", "2026-09-20 00:00:00"),
            ("2026-09-20 00:00:00", "2026-09-20 00:00:00"),
        ]
        .map(|(start, end)| (start.to_owned(), end.to_owned()))
    );
}

#[test]
fn gantt_resolves_forward_references_implicit_starts_and_until_across_leap_day() {
    let source = "gantt
        Forward :later, after start, 1w
        Start :start, 2028-02-28, 2d
        Follow :1d
        Until :2028-02-27, until later";
    let chart = Chart::parse(source).expect("valid schedule");
    let dates: Vec<_> = chart
        .schedule()
        .expect("resolved dates")
        .into_iter()
        .map(|task| (task.start.to_string(), task.end.to_string()))
        .collect();
    assert_eq!(
        dates,
        [
            ("2028-03-01 00:00:00", "2028-03-08 00:00:00"),
            ("2028-02-28 00:00:00", "2028-03-01 00:00:00"),
            ("2028-03-01 00:00:00", "2028-03-02 00:00:00"),
            ("2028-02-27 00:00:00", "2028-03-01 00:00:00"),
        ]
        .map(|(start, end)| (start.to_owned(), end.to_owned()))
    );
}

#[test]
fn gantt_renders_statuses_unicode_and_milestone_midpoints_within_width() {
    let source = "gantt
        axisFormat %m-%d
        section Release
        設計 :done, a, 2026-09-09, 2d
        Build :active, crit, after a, 4d
        Midpoint :milestone, 2026-09-09, 6d";
    let output = render(source, 64).expect("rendered schedule");
    assert!(output.iter().all(|line| line.width() <= 64));
    insta::assert_snapshot!(
        output
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        render(source, 24).is_none(),
        "preserve source when labels and axis cannot fit"
    );
}

#[test]
fn gantt_preserves_source_for_unresolvable_or_unsupported_schedules() {
    for source in [
        "gantt\nA :a, after b, 1d\nB :b, after a, 1d",
        "gantt\nA :a, after missing, 1d",
        "gantt\nA :a, 2026-09-09, 1d\nB :a, after a, 1d",
        "gantt\nA :2026-02-30, 1d",
        "gantt\nA :2026-09-09, 2026-09-08",
        "gantt\nA :1d",
        "gantt\nexcludes weekends\nA :2026-09-09, 1d",
        "gantt\ndateFormat DD-MM-YYYY\nA :09-09-2026, 1d",
        "gantt\naxisFormat %Q\nA :2026-09-09, 1d",
        "gantt\naxisFormat %\nA :2026-09-09, 1d",
        "gantt\nA :2026-09-09, 1M",
        "gantt\nA :2026-09-09, 999999999999999999999999d",
        "gantt\nA :2026-09-09, 999999d",
        "gantt\nA :2026-09-09, until missing",
        "gantt\nsection Empty",
    ] {
        assert!(
            render(source, 100).is_none(),
            "unexpected rendering: {source}"
        );
    }
}
