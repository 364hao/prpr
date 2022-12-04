use super::process_lines;
use crate::{
    core::{
        AnimFloat, AnimVector, Chart, JudgeLine, JudgeLineKind, Keyframe, Note, NoteKind, Object,
        HEIGHT_RATIO, NOTE_WIDTH_RATIO,
    },
    ext::NotNanExt,
};
use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PgrEvent {
    pub start_time: f32,
    pub end_time: f32,
    pub start: f32,
    pub end: f32,
    pub start2: f32,
    pub end2: f32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PgrSpeedEvent {
    pub start_time: f32,
    pub end_time: f32,
    pub value: f32,
    pub floor_position: f32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PgrNote {
    #[serde(rename = "type")]
    kind: u8,
    time: f32,
    position_x: f32,
    hold_time: f32,
    speed: f32,
    floor_position: f32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PgrJudgeLine {
    bpm: f32,
    #[serde(rename = "judgeLineDisappearEvents")]
    alpha_events: Vec<PgrEvent>,
    #[serde(rename = "judgeLineRotateEvents")]
    rotate_events: Vec<PgrEvent>,
    #[serde(rename = "judgeLineMoveEvents")]
    move_events: Vec<PgrEvent>,
    speed_events: Vec<PgrSpeedEvent>,

    notes_above: Vec<PgrNote>,
    notes_below: Vec<PgrNote>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PgrChart {
    offset: f32,
    judge_line_list: Vec<PgrJudgeLine>,
}

macro_rules! validate_events {
    ($pgr:expr) => {
        if $pgr.iter().any(|it| it.start_time > it.end_time) {
            bail!("Invalid time range");
        }
        for i in 0..($pgr.len() - 1) {
            if $pgr[i].end_time != $pgr[i + 1].start_time {
                bail!("Events should be contiguous");
            }
        }
        if $pgr.last().unwrap().end_time <= 900000000.0 {
            bail!(
                "End time is not great enough ({})",
                $pgr.last().unwrap().end_time
            );
        }
    };
}

fn parse_speed_events(r: f32, pgr: Vec<PgrSpeedEvent>, max_time: f32) -> Result<AnimFloat> {
    validate_events!(pgr);
    assert_eq!(pgr[0].start_time, 0.0);
    let mut kfs = Vec::new();
    kfs.extend(
        pgr.iter()
            .map(|it| Keyframe::new(it.start_time * r, it.floor_position, 2)),
    );
    let last = pgr.last().unwrap();
    kfs.push(Keyframe::new(
        max_time,
        last.floor_position + (max_time - last.start_time * r) * last.value,
        0,
    ));
    for kf in &mut kfs {
        kf.value /= HEIGHT_RATIO;
    }
    Ok(AnimFloat::new(kfs))
}

fn parse_float_events(r: f32, pgr: Vec<PgrEvent>) -> Result<AnimFloat> {
    validate_events!(pgr);
    let mut kfs = Vec::<Keyframe<f32>>::new();
    for e in pgr {
        if !kfs.last().map(|it| it.value == e.start).unwrap_or_default() {
            kfs.push(Keyframe::new((e.start_time * r).max(0.), e.start, 2));
        }
        kfs.push(Keyframe::new(e.end_time * r, e.end, 2));
    }
    kfs.pop();
    Ok(AnimFloat::new(kfs))
}

fn parse_move_events(r: f32, pgr: Vec<PgrEvent>) -> Result<AnimVector> {
    validate_events!(pgr);
    let mut kf1 = Vec::<Keyframe<f32>>::new();
    let mut kf2 = Vec::<Keyframe<f32>>::new();
    for e in pgr {
        let st = (e.start_time * r).max(0.);
        let en = e.end_time * r;
        if !kf1.last().map(|it| it.value == e.start).unwrap_or_default() {
            kf1.push(Keyframe::new(st, e.start, 2));
        }
        if !kf2
            .last()
            .map(|it| it.value == e.start2)
            .unwrap_or_default()
        {
            kf2.push(Keyframe::new(st, e.start2, 2));
        }
        kf1.push(Keyframe::new(en, e.end, 2));
        kf2.push(Keyframe::new(en, e.end2, 2));
    }
    kf1.pop();
    kf2.pop();
    for kf in &mut kf1 {
        kf.value = -1. + kf.value * 2.;
    }
    for kf in &mut kf2 {
        kf.value = -1. + kf.value * 2.;
    }
    Ok(AnimVector(AnimFloat::new(kf1), AnimFloat::new(kf2)))
}

fn parse_notes(r: f32, pgr: Vec<PgrNote>, height: &mut AnimFloat) -> Result<Vec<Note>> {
    // is_sorted is unstable...
    if pgr.is_empty() {
        return Ok(Vec::new());
    }
    for i in 0..(pgr.len() - 1) {
        if pgr[i].time > pgr[i + 1].time {
            bail!("Notes are not sorted");
        }
    }
    pgr.into_iter()
        .map(|pgr| {
            Ok(Note {
                object: Object {
                    translation: AnimVector(
                        AnimFloat::fixed(pgr.position_x * NOTE_WIDTH_RATIO),
                        AnimFloat::default(),
                    ),
                    ..Default::default()
                },
                kind: match pgr.kind {
                    1 => NoteKind::Click,
                    2 => NoteKind::Drag,
                    3 => {
                        let end_time = (pgr.time + pgr.hold_time) * r;
                        height.set_time(end_time);
                        let end_height = height.now();
                        NoteKind::Hold {
                            end_time,
                            end_height,
                        }
                    }
                    4 => NoteKind::Flick,
                    _ => bail!("Unknown note type: {}", pgr.kind),
                },
                time: pgr.time * r,
                speed: pgr.speed, // TODO this is not right
                height: pgr.floor_position / HEIGHT_RATIO,
                multiple_hint: false,
                fake: false,
                last_real_time: 0.0,
            })
        })
        .collect()
}

fn parse_judge_line(pgr: PgrJudgeLine, max_time: f32) -> Result<JudgeLine> {
    let r = 60. / pgr.bpm / 32.;
    let mut height = parse_speed_events(r, pgr.speed_events, max_time)
        .context("Failed to parse speed events")?;
    let notes_above =
        parse_notes(r, pgr.notes_above, &mut height).context("Failed to parse notes above")?;
    let notes_below =
        parse_notes(r, pgr.notes_below, &mut height).context("Failed to parse notes below")?;
    Ok(JudgeLine {
        object: Object {
            alpha: parse_float_events(r, pgr.alpha_events)
                .context("Failed to parse alpha events")?,
            rotation: parse_float_events(r, pgr.rotate_events)
                .context("Failed to parse rotate events")?,
            translation: parse_move_events(r, pgr.move_events)
                .context("Failed to parse move events")?,
            ..Default::default()
        },
        kind: JudgeLineKind::Normal,
        height,
        notes_above,
        notes_below,
        parent: None,
        show_below: true,
    })
}

pub fn parse_phigros(source: &str) -> Result<Chart> {
    let pgr: PgrChart = serde_json::from_str(source).context("Failed to parse JSON")?;
    let max_time = *pgr
        .judge_line_list
        .iter()
        .map(|line| {
            line.notes_above
                .iter()
                .chain(line.notes_below.iter())
                .map(|note| note.time.not_nan())
                .max()
                .unwrap_or_default()
                * (60. / line.bpm / 32.)
        })
        .max()
        .unwrap_or_default()
        + 1.;
    let mut lines = pgr
        .judge_line_list
        .into_iter()
        .enumerate()
        .map(|(id, pgr)| {
            parse_judge_line(pgr, max_time).with_context(|| format!("In judge line #{id}"))
        })
        .collect::<Result<Vec<_>>>()?;
    process_lines(&mut lines);
    Ok(Chart {
        offset: pgr.offset,
        lines,
    })
}
