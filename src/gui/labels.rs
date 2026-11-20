use super::{decision, paths};
use gpui::{Pixels, SharedString, px};
use unicode_segmentation::UnicodeSegmentation;

type Measure<'a> = &'a dyn Fn(&SharedString) -> Pixels;

fn width(label: &str, measure: Measure<'_>) -> Pixels {
    measure(&SharedString::from(label.to_owned()))
}

fn context(text: &str, start: usize, end: usize) -> String {
    let before = if start == 0 { "" } else { "…" };
    let after = if end == text.len() { "" } else { "…" };
    format!("{before}{}{after}", &text[start..end])
}

fn fit_context(
    text: &str,
    cue: &paths::LabelCue,
    room: Pixels,
    measure: Measure<'_>,
) -> Option<String> {
    let mut boundaries = text
        .split_word_bound_indices()
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    boundaries.push(text.len());
    let start = boundaries.partition_point(|&offset| offset <= cue.span.start) - 1;
    let end = boundaries
        .partition_point(|&offset| offset < cue.span.end)
        .max((start + 1).min(boundaries.len() - 1));
    let word = context(text, boundaries[start], boundaries[end]);
    if width(&word, measure) > room {
        let precise = context(text, cue.span.start, cue.span.end);
        return (!cue.span.is_empty() && width(&precise, measure) <= room).then_some(precise);
    }

    let mut label = word;
    let mut low = 1;
    let mut high = start + boundaries.len() - 1 - end;
    while low <= high {
        let extra = low + (high - low) / 2;
        let left = (extra / 2)
            .min(start)
            .max(extra.saturating_sub(boundaries.len() - 1 - end));
        let candidate = context(
            text,
            boundaries[start - left],
            boundaries[end + extra - left],
        );
        if width(&candidate, measure) <= room {
            label = candidate;
            low = extra + 1;
        } else {
            high = extra - 1;
        }
    }
    Some(label)
}

pub(super) fn filename(
    name: &str,
    suffix: &str,
    cue: Option<&paths::LabelCue>,
    room: Pixels,
    measure: Measure<'_>,
) -> String {
    let full = format!("{name}{suffix}");
    if width(&full, measure) <= room {
        return full;
    }
    let (stem, ending) = name
        .rfind('.')
        .filter(|&split| split > 0 && name.len() - split <= 9)
        .filter(|&split| cue.is_none_or(|cue| cue.span.end <= split))
        .map_or((name, suffix.to_owned()), |split| {
            (&name[..split], format!("{}{suffix}", &name[split..]))
        });
    let space = room - width(&ending, measure);
    let label = if let Some(cue) = cue {
        fit_context(stem, cue, space, measure).unwrap_or_else(|| {
            let number = format!("#{} ", cue.ordinal);
            if width(&number, measure) + width("…", measure) >= space {
                return number.trim_end().to_owned();
            }
            let label =
                decision::abbreviate_folder(stem, space - width(&number, measure), &measure);
            format!("{number}{label}")
        })
    } else {
        decision::abbreviate_folder(stem, space, &measure).to_string()
    };
    format!("{label}{ending}")
}

pub(super) fn document(
    file: &str,
    suffix: &str,
    folder: &paths::ParentHint,
    separator: &str,
    room: Pixels,
    measure: Measure<'_>,
) -> String {
    let full = format!("{file}{suffix}{separator}{}", folder.text);
    if width(&full, measure) <= room {
        return full;
    }
    let number = format!("#{}", folder.cue.ordinal);
    let available = room - width(separator, measure);
    let reserve = (available * 0.34).max(width(&number, measure));
    let file = filename(file, suffix, None, available - reserve, measure);
    let room = (available - width(&file, measure)).max(px(0.));
    let hint = if width(&folder.text, measure) <= room {
        folder.text.clone()
    } else {
        fit_context(&folder.text, &folder.cue, room, measure).unwrap_or(number)
    };
    format!("{file}{separator}{hint}")
}
