//! Subtitle pipeline — in-memory representations and pure-logic operations.
//!
//! This module covers the I/O-free side of subtitle handling: parsing both
//! supported wire formats into typed data models, the operations the schema
//! exposes (`filter`, `subtract_track`, format conversion), and serialization
//! back out. It deliberately stops short of anything that touches the
//! filesystem or external processes — those live in [`crate::pipeline`].
//!
//! Implemented:
//!
//! - **SRT** — [`Srt`] / [`SrtEvent`] / [`SrtTime`] data model;
//!   [`parse_srt`] (tolerant of BOM, CRLF/LF, missing index numbers, trailing
//!   whitespace, multi-line event text); [`serialize_srt`] (canonical output
//!   with sequential indices); [`subtract_by_timestamp`].
//! - **ASS** — [`Ass`] / [`AssStyle`] / [`AssEvent`] / [`AssTime`] data model;
//!   [`parse_ass`]; [`serialize_ass`]; [`subtract_ass_by_timestamp`];
//!   [`filter_ass`] (style/font matching with retain/remove semantics);
//!   [`ass_to_srt`] (lossy plain-text conversion).
//!
//! Not handled here — lives elsewhere in the pipeline:
//!
//! - **Extraction** of subtitle tracks from source MKVs via ffmpeg —
//!   [`crate::pipeline::subtitle_prep`].
//! - **Burn rendering** of subtitle tracks onto the video stream via libass,
//!   driven through ffmpeg's `subtitles=` filter.
//! - **Soft mux** of derived tracks into the output container with their
//!   declared dispositions (lang, default, forced, etc.).
//! - **Integration** into the convert command's per-file flow
//!   ([`crate::pipeline::run_convert`]).
//!
//! The canonical anime case from `DESIGN.md > [subtitles]` is "full dialogue
//! track minus signs-only track = soft dialogue-only track plus burned signs."
//! Every operation needed to compute that derivation lives in this module;
//! the remaining work is wiring it into the ffmpeg invocation.

use std::collections::{HashMap, HashSet};

use thiserror::Error;

use crate::config::{FilterMode, SubtitleFilter};

// =============================================================================
// Data model
// =============================================================================

/// A parsed SRT subtitle file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Srt {
    pub events: Vec<SrtEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrtEvent {
    pub start: SrtTime,
    pub end: SrtTime,
    /// Multi-line text as written; internal newlines preserved verbatim.
    pub text: String,
}

/// Millisecond-precise timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SrtTime {
    pub millis: u64,
}

impl SrtTime {
    pub fn from_millis(millis: u64) -> Self {
        Self { millis }
    }
}

#[derive(Debug, Clone, Error)]
#[error("SRT parse error at line {line}: {message}")]
pub struct SrtParseError {
    pub line: usize,
    pub message: String,
}

// =============================================================================
// Parsing
// =============================================================================

/// Parse an SRT subtitle file into a structured [`Srt`].
///
/// Tolerant: strips a leading BOM, accepts CRLF or LF line endings, treats
/// the index number as optional (since some tools omit it), allows extra
/// blank lines between events, and accepts both `,` and `.` as the
/// fractional-second separator (some SRT producers use `.`).
pub fn parse_srt(input: &str) -> Result<Srt, SrtParseError> {
    let input = input.strip_prefix('\u{FEFF}').unwrap_or(input);

    // Pre-collect lines with 1-based numbers so error messages are useful.
    let lines: Vec<(usize, &str)> = input
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l))
        .collect();

    let mut events = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        // Skip blank lines.
        while i < lines.len() && lines[i].1.trim().is_empty() {
            i += 1;
        }
        if i >= lines.len() {
            break;
        }

        // The current line is either the optional index or the timestamp.
        let (line_no, line_text) = lines[i];
        let (timestamp_line_no, timestamp_text) = if is_timestamp_line(line_text) {
            (line_no, line_text)
        } else {
            // Treat current line as the index; advance to the timestamp.
            i += 1;
            if i >= lines.len() {
                return Err(SrtParseError {
                    line: line_no,
                    message: "expected timestamp line after event index".into(),
                });
            }
            lines[i]
        };

        let (start, end) = parse_timestamp_line(timestamp_text).ok_or_else(|| SrtParseError {
            line: timestamp_line_no,
            message: format!("malformed timestamp line: {:?}", timestamp_text),
        })?;
        i += 1;

        // Read text lines until blank line or EOF.
        let mut text_lines = Vec::new();
        while i < lines.len() && !lines[i].1.trim().is_empty() {
            text_lines.push(lines[i].1);
            i += 1;
        }

        events.push(SrtEvent {
            start,
            end,
            text: text_lines.join("\n"),
        });
    }

    Ok(Srt { events })
}

fn is_timestamp_line(line: &str) -> bool {
    line.contains("-->")
}

fn parse_timestamp_line(line: &str) -> Option<(SrtTime, SrtTime)> {
    let mut parts = line.splitn(2, "-->");
    let start_str = parts.next()?.trim();
    let rest = parts.next()?.trim();
    // The end may have positional metadata after it on some SRT producers
    // (e.g. "X1:... Y1:..."); use only the first whitespace-delimited token.
    let end_str = rest.split_whitespace().next()?;
    Some((parse_srt_time(start_str)?, parse_srt_time(end_str)?))
}

fn parse_srt_time(s: &str) -> Option<SrtTime> {
    let s = s.trim();
    let (hms, ms_str) = match s.rfind(|c| c == ',' || c == '.') {
        Some(idx) => (&s[..idx], &s[idx + 1..]),
        None => return None,
    };
    let mut parts = hms.split(':');
    let h: u64 = parts.next()?.parse().ok()?;
    let m: u64 = parts.next()?.parse().ok()?;
    let sec: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let ms: u64 = ms_str.parse().ok()?;
    if m >= 60 || sec >= 60 || ms >= 1000 {
        return None;
    }
    Some(SrtTime::from_millis(
        ((h * 3600 + m * 60 + sec) * 1000) + ms,
    ))
}

// =============================================================================
// Serialization
// =============================================================================

/// Serialize an [`Srt`] back to canonical SRT text.
///
/// - LF line endings.
/// - 1-based sequential event indices, regardless of any indices on the input.
/// - Trailing blank line after the final event (per SRT convention).
pub fn serialize_srt(srt: &Srt) -> String {
    let mut out = String::new();
    for (i, event) in srt.events.iter().enumerate() {
        out.push_str(&(i + 1).to_string());
        out.push('\n');
        out.push_str(&format_srt_time(event.start));
        out.push_str(" --> ");
        out.push_str(&format_srt_time(event.end));
        out.push('\n');
        out.push_str(&event.text);
        out.push_str("\n\n");
    }
    out
}

fn format_srt_time(t: SrtTime) -> String {
    let total_ms = t.millis;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    format!("{:02}:{:02}:{:02},{:03}", h, m, s, ms)
}

// =============================================================================
// Derivation: subtract_track
// =============================================================================

/// Drop events from `base` whose `(start, end)` timestamp pair exactly matches
/// an event in `subtrahend`. Implements the SRT case of the
/// `subtract_track` derivation (DESIGN.md > [subtitles] > Per-track fields —
/// derivation).
///
/// Matching is strictly on the `(start, end)` pair as a unit. Partial overlaps
/// and timestamp shifts are not matched — the intended use is the canonical
/// "full minus signs = dialogue-only" case where the signs track is a literal
/// subset of the full track with identical timing.
pub fn subtract_by_timestamp(base: &Srt, subtrahend: &Srt) -> Srt {
    let drop_pairs: HashSet<(SrtTime, SrtTime)> = subtrahend
        .events
        .iter()
        .map(|e| (e.start, e.end))
        .collect();

    Srt {
        events: base
            .events
            .iter()
            .filter(|e| !drop_pairs.contains(&(e.start, e.end)))
            .cloned()
            .collect(),
    }
}

// =============================================================================
// ASS — data model
// =============================================================================

/// A parsed ASS (Advanced SubStation Alpha) subtitle file.
///
/// The model is split: `header` is the verbatim text of every section other
/// than `[Events]` (preserved for round-trip), `events` is the parsed event
/// list, and `styles` is a parsed map from style name to style attributes
/// (font name, size) used by [`filter_ass`] for font/size matching. The
/// styles map is built *alongside* the verbatim header — re-serializing
/// emits the original style text from `header`, not a re-rendering of the
/// parsed map.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ass {
    /// All sections preceding `[Events]`, kept verbatim.
    pub header: String,
    /// The `Format:` line from `[Events]`, kept verbatim so column ordering
    /// in the round-trip matches the input.
    pub events_format: String,
    pub events: Vec<AssEvent>,
    /// Parsed `[V4+ Styles]` (or `[V4 Styles]`) entries, keyed by style name.
    /// Populated as a side product of parsing — only fields needed for filter
    /// matching are extracted (others remain in `header` verbatim).
    pub styles: HashMap<String, AssStyle>,
}

/// A subset of an ASS style definition — only the fields the filter operation
/// uses. Other style fields (colors, alignment, margins, etc.) stay in the
/// verbatim header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssStyle {
    pub name: String,
    /// As written in the file. Use [`normalize_font_name`] for comparison —
    /// the `@` prefix used by some tools for vertical writing is stripped.
    pub fontname: String,
    pub fontsize: u32,
}

/// Strip the leading `@` (used by some ASS authoring tools for vertical
/// writing of the same family) so that user filter `font = "Arial"` matches
/// styles whose `Fontname` field is `@Arial`.
fn normalize_font_name(name: &str) -> &str {
    name.trim_start_matches('@')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssEvent {
    /// Original line, kept verbatim for round-trip emission. Subtract and
    /// filter only drop events; they don't modify them, so `raw` stays valid.
    pub raw: String,
    /// `Dialogue` or `Comment` (or other event-line prefixes — kept as a
    /// string so unusual sources don't error).
    pub kind: String,
    pub start: AssTime,
    pub end: AssTime,
    pub style: String,
    /// Raw text including ASS override tags (`{\an8}` etc.). Not modified by
    /// subtract or filter; stripped only by [`ass_to_srt`].
    pub text: String,
}

/// Millisecond-precise timestamp. ASS files store centisecond precision on
/// disk; the millisecond unit here matches [`SrtTime`] for trivial conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssTime {
    pub millis: u64,
}

impl AssTime {
    pub fn from_millis(millis: u64) -> Self {
        Self { millis }
    }
}

#[derive(Debug, Clone, Error)]
#[error("ASS parse error at line {line}: {message}")]
pub struct AssParseError {
    pub line: usize,
    pub message: String,
}

// =============================================================================
// ASS — parsing
// =============================================================================

/// Parse an ASS file into a structured [`Ass`].
///
/// Tolerant: strips a leading BOM, accepts CRLF or LF line endings, ignores
/// blank lines and comment-style entries within `[Events]`, and accepts both
/// `Dialogue:` and `Comment:` event-line prefixes.
///
/// The standard ASS event format has 10 fields:
/// `Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text`.
/// We assume that ordering. The `Text` field can contain commas, so we split
/// the event line into exactly 10 parts and treat the 10th as the full text.
pub fn parse_ass(input: &str) -> Result<Ass, AssParseError> {
    let input = input.strip_prefix('\u{FEFF}').unwrap_or(input);

    let mut header = String::new();
    let mut events_format = String::new();
    let mut events = Vec::new();
    let mut styles: HashMap<String, AssStyle> = HashMap::new();
    let mut in_events_section = false;
    let mut in_styles_section = false;
    let mut style_format: Option<StyleColumns> = None;

    for (idx, line) in input.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed_left = line.trim_start();

        // Section header line: `[Section Name]`.
        if trimmed_left.starts_with('[') && trimmed_left.contains(']') {
            // Extract the inner section label between `[` and the first `]`.
            let inner = trimmed_left
                .trim_start_matches('[')
                .split(']')
                .next()
                .unwrap_or("")
                .trim();
            in_events_section = inner == "Events";
            // Match `V4 Styles`, `V4+ Styles`, `V4++ Styles`, etc.
            in_styles_section = inner.starts_with("V4") && inner.ends_with("Styles");
            if in_styles_section {
                // Format declaration is local to each styles section.
                style_format = None;
            }
            if !in_events_section {
                header.push_str(line);
                header.push('\n');
            }
            continue;
        }

        if !in_events_section {
            header.push_str(line);
            header.push('\n');

            if in_styles_section {
                // Format / Style entries are extracted into `styles` but the
                // raw text still lives in `header` for verbatim round-trip.
                if let Some(rest) = trimmed_left.strip_prefix("Format:") {
                    style_format = StyleColumns::parse(rest);
                } else if let Some(rest) = trimmed_left.strip_prefix("Style:") {
                    if let Some(cols) = &style_format {
                        if let Some(style) = parse_style_line(rest, cols) {
                            styles.insert(style.name.clone(), style);
                        }
                    }
                }
            }
            continue;
        }

        // Inside [Events].
        if trimmed_left.starts_with("Format:") {
            events_format = line.to_string();
            continue;
        }
        if let Some(rest) = trimmed_left
            .strip_prefix("Dialogue:")
            .or_else(|| trimmed_left.strip_prefix("Comment:"))
        {
            let kind = if trimmed_left.starts_with("Dialogue:") {
                "Dialogue"
            } else {
                "Comment"
            };
            events.push(parse_ass_event_line(rest, kind, line, line_no)?);
        }
        // Blank lines and other content within [Events] are ignored.
    }

    Ok(Ass {
        header,
        events_format,
        events,
        styles,
    })
}

/// Column indices into a `[V4+ Styles]` row, derived from the section's
/// `Format:` declaration. Honors arbitrary column orderings — only the three
/// columns the filter operation needs are tracked.
struct StyleColumns {
    name: usize,
    fontname: usize,
    fontsize: usize,
}

impl StyleColumns {
    fn parse(format_rest: &str) -> Option<Self> {
        let cols: Vec<&str> = format_rest.split(',').map(|s| s.trim()).collect();
        Some(Self {
            name: cols.iter().position(|c| c.eq_ignore_ascii_case("Name"))?,
            fontname: cols.iter().position(|c| c.eq_ignore_ascii_case("Fontname"))?,
            fontsize: cols.iter().position(|c| c.eq_ignore_ascii_case("Fontsize"))?,
        })
    }
}

fn parse_style_line(rest: &str, cols: &StyleColumns) -> Option<AssStyle> {
    let parts: Vec<&str> = rest.split(',').map(|s| s.trim()).collect();
    let name = parts.get(cols.name)?.to_string();
    let fontname = parts.get(cols.fontname)?.to_string();
    let fontsize: u32 = parts.get(cols.fontsize)?.parse().ok()?;
    Some(AssStyle {
        name,
        fontname,
        fontsize,
    })
}

fn parse_ass_event_line(
    rest: &str,
    kind: &str,
    raw: &str,
    line_no: usize,
) -> Result<AssEvent, AssParseError> {
    let parts: Vec<&str> = rest.splitn(10, ',').collect();
    if parts.len() < 10 {
        return Err(AssParseError {
            line: line_no,
            message: format!(
                "{} line has {} comma-separated fields, expected 10",
                kind,
                parts.len(),
            ),
        });
    }
    let start = parse_ass_time(parts[1].trim()).ok_or_else(|| AssParseError {
        line: line_no,
        message: format!("malformed start timestamp: {:?}", parts[1]),
    })?;
    let end = parse_ass_time(parts[2].trim()).ok_or_else(|| AssParseError {
        line: line_no,
        message: format!("malformed end timestamp: {:?}", parts[2]),
    })?;
    let style = parts[3].trim().to_string();
    let text = parts[9].to_string();

    Ok(AssEvent {
        raw: raw.to_string(),
        kind: kind.to_string(),
        start,
        end,
        style,
        text,
    })
}

fn parse_ass_time(s: &str) -> Option<AssTime> {
    let s = s.trim();
    let (hms, cs_str) = s.rsplit_once('.')?;
    let mut parts = hms.split(':');
    let h: u64 = parts.next()?.parse().ok()?;
    let m: u64 = parts.next()?.parse().ok()?;
    let sec: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let cs: u64 = cs_str.parse().ok()?;
    if m >= 60 || sec >= 60 || cs >= 100 {
        return None;
    }
    Some(AssTime::from_millis(
        (h * 3600 + m * 60 + sec) * 1000 + cs * 10,
    ))
}

// =============================================================================
// ASS — serialization
// =============================================================================

/// Serialize an [`Ass`] back to ASS text. The header is written verbatim,
/// then the `[Events]` section is reconstructed from `events_format` plus the
/// `raw` field of each retained event.
pub fn serialize_ass(ass: &Ass) -> String {
    let mut out = String::new();
    out.push_str(&ass.header);
    if !ass.header.is_empty() && !ass.header.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("[Events]\n");
    if !ass.events_format.is_empty() {
        out.push_str(&ass.events_format);
        out.push('\n');
    }
    for event in &ass.events {
        out.push_str(&event.raw);
        out.push('\n');
    }
    out
}

// =============================================================================
// ASS — derivation: subtract_track
// =============================================================================

/// Drop events from `base` whose `(start, end)` timestamp pair exactly
/// matches an event in `subtrahend`. ASS analogue of [`subtract_by_timestamp`].
pub fn subtract_ass_by_timestamp(base: &Ass, subtrahend: &Ass) -> Ass {
    let drop_pairs: HashSet<(AssTime, AssTime)> = subtrahend
        .events
        .iter()
        .map(|e| (e.start, e.end))
        .collect();

    Ass {
        header: base.header.clone(),
        events_format: base.events_format.clone(),
        styles: base.styles.clone(),
        events: base
            .events
            .iter()
            .filter(|e| !drop_pairs.contains(&(e.start, e.end)))
            .cloned()
            .collect(),
    }
}

// =============================================================================
// ASS — derivation: filter (style / font / size match)
// =============================================================================

/// Keep or drop events based on a [`SubtitleFilter`]. Match keys (`style`,
/// `font`, `size`) are AND-ed; an unset key matches anything; `mode = Retain`
/// keeps matching events, `mode = Remove` drops them. A filter without `mode`
/// is a no-op (`base` returned unchanged).
///
/// `style` is matched against the event's own style name. `font` and `size`
/// are matched against the *style-defined* values (looked up in `base.styles`
/// from the `[V4+ Styles]` section); inline override tags like
/// `{\fnComicSans\fs60}` that change font/size mid-event aren't inspected —
/// for the fansub workflows this targets, styles are purpose-built and
/// override tags are used for inline emphasis, so style-defined matching is
/// the right level of granularity.
///
/// Font names are normalized via [`normalize_font_name`] for comparison so
/// `font = "Arial"` matches a style whose `Fontname` is `@Arial` (the prefix
/// some authoring tools add for vertical-writing variants of the same family).
///
/// Events whose style is not present in `base.styles` (orphan style
/// references, or an ASS file with no `[V4+ Styles]` section) cannot have
/// their font/size resolved and are treated as non-matching for those keys.
/// A `style`-only filter still works the same as before.
pub fn filter_ass(base: &Ass, filter: &SubtitleFilter) -> Ass {
    let Some(mode) = filter.mode else {
        return base.clone();
    };
    let retain = matches!(mode, FilterMode::Retain);

    let style_filter = filter.style.as_deref();
    let font_filter = filter.font.as_deref().map(normalize_font_name);
    let size_filter = filter.size;

    Ass {
        header: base.header.clone(),
        events_format: base.events_format.clone(),
        styles: base.styles.clone(),
        events: base
            .events
            .iter()
            .filter(|event| {
                let matches = event_matches_filter(
                    event,
                    &base.styles,
                    style_filter,
                    font_filter,
                    size_filter,
                );
                if retain { matches } else { !matches }
            })
            .cloned()
            .collect(),
    }
}

fn event_matches_filter(
    event: &AssEvent,
    styles: &HashMap<String, AssStyle>,
    style_filter: Option<&str>,
    font_filter: Option<&str>,
    size_filter: Option<u32>,
) -> bool {
    if let Some(s) = style_filter {
        if event.style != s {
            return false;
        }
    }
    if font_filter.is_some() || size_filter.is_some() {
        // Font/size live on the style definition. Orphan style → no match.
        let Some(resolved) = styles.get(&event.style) else {
            return false;
        };
        if let Some(f) = font_filter {
            if normalize_font_name(&resolved.fontname) != f {
                return false;
            }
        }
        if let Some(sz) = size_filter {
            if resolved.fontsize != sz {
                return false;
            }
        }
    }
    true
}

// =============================================================================
// ASS → SRT (lossy plain-text conversion)
// =============================================================================

/// Convert an ASS file to SRT, stripping override tags (`{\…}`) and
/// translating ASS line-break escapes (`\N`, `\n`, `\h`) to plain text.
///
/// Lossy: styling, positioning, fonts, and effects are stripped. The doc
/// warns about this conversion via `[subtitles].warn_ass_to_srt`; this
/// function is the underlying operation.
pub fn ass_to_srt(ass: &Ass) -> Srt {
    Srt {
        events: ass
            .events
            .iter()
            .filter(|e| e.kind == "Dialogue") // Comments aren't displayed.
            .map(|e| SrtEvent {
                start: SrtTime::from_millis(e.start.millis),
                end: SrtTime::from_millis(e.end.millis),
                text: ass_text_to_plain(&e.text),
            })
            .filter(|e| !e.text.trim().is_empty())
            .collect(),
    }
}

/// Strip ASS override tags and translate line-break escapes.
fn ass_text_to_plain(text: &str) -> String {
    // Strip everything between `{` and `}` (override tags).
    let mut out = String::new();
    let mut depth: u32 = 0;
    for c in text.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    // ASS line-break escapes.
    // \N — hard line break
    // \n — soft line break (typically rendered as space)
    // \h — non-breaking space
    out.replace("\\N", "\n").replace("\\n", " ").replace("\\h", " ")
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u64, m: u64, s: u64, ms: u64) -> SrtTime {
        SrtTime::from_millis(((h * 3600 + m * 60 + s) * 1000) + ms)
    }

    // --- Time parsing -------------------------------------------------------

    #[test]
    fn parses_canonical_timestamp() {
        assert_eq!(parse_srt_time("00:00:01,234"), Some(SrtTime::from_millis(1234)));
        assert_eq!(parse_srt_time("01:02:03,456"), Some(t(1, 2, 3, 456)));
    }

    #[test]
    fn accepts_dot_as_fractional_separator() {
        assert_eq!(parse_srt_time("00:00:01.500"), Some(SrtTime::from_millis(1500)));
    }

    #[test]
    fn rejects_malformed_timestamps() {
        assert_eq!(parse_srt_time(""), None);
        assert_eq!(parse_srt_time("00:00:01"), None); // no fractional
        assert_eq!(parse_srt_time("00:60:00,000"), None); // invalid minutes
        assert_eq!(parse_srt_time("00:00:60,000"), None); // invalid seconds
        assert_eq!(parse_srt_time("00:00:00,1000"), None); // ms out of range
    }

    // --- Timestamp formatting -----------------------------------------------

    #[test]
    fn formats_canonical_timestamp() {
        assert_eq!(format_srt_time(SrtTime::from_millis(1234)), "00:00:01,234");
        assert_eq!(format_srt_time(t(1, 2, 3, 456)), "01:02:03,456");
        assert_eq!(format_srt_time(SrtTime::from_millis(0)), "00:00:00,000");
    }

    // --- Single-event parsing -----------------------------------------------

    const SAMPLE_ONE_EVENT: &str = "\
1
00:00:01,000 --> 00:00:04,500
Hello, world.
";

    #[test]
    fn parses_single_event() {
        let srt = parse_srt(SAMPLE_ONE_EVENT).unwrap();
        assert_eq!(srt.events.len(), 1);
        assert_eq!(srt.events[0].start, SrtTime::from_millis(1000));
        assert_eq!(srt.events[0].end, SrtTime::from_millis(4500));
        assert_eq!(srt.events[0].text, "Hello, world.");
    }

    // --- Multi-event parsing ------------------------------------------------

    const SAMPLE_THREE_EVENTS: &str = "\
1
00:00:01,000 --> 00:00:04,500
First subtitle.
This continues on a second line.

2
00:00:05,000 --> 00:00:07,250
Second subtitle.

3
00:00:09,100 --> 00:00:11,000
Third subtitle.
";

    #[test]
    fn parses_multiple_events_with_multi_line_text() {
        let srt = parse_srt(SAMPLE_THREE_EVENTS).unwrap();
        assert_eq!(srt.events.len(), 3);
        assert_eq!(srt.events[0].text, "First subtitle.\nThis continues on a second line.");
        assert_eq!(srt.events[1].text, "Second subtitle.");
        assert_eq!(srt.events[2].text, "Third subtitle.");
    }

    // --- Tolerance: BOM, CRLF, missing index, blank-line padding ------------

    #[test]
    fn strips_bom() {
        let with_bom = format!("\u{FEFF}{}", SAMPLE_ONE_EVENT);
        assert_eq!(parse_srt(&with_bom).unwrap().events.len(), 1);
    }

    #[test]
    fn accepts_crlf_line_endings() {
        let crlf = SAMPLE_THREE_EVENTS.replace('\n', "\r\n");
        let srt = parse_srt(&crlf).unwrap();
        assert_eq!(srt.events.len(), 3);
        // Multi-line text should not contain stray \r characters.
        assert_eq!(srt.events[0].text, "First subtitle.\nThis continues on a second line.");
    }

    #[test]
    fn accepts_missing_index_numbers() {
        let no_indices = "\
00:00:01,000 --> 00:00:04,500
First.

00:00:05,000 --> 00:00:07,250
Second.
";
        let srt = parse_srt(no_indices).unwrap();
        assert_eq!(srt.events.len(), 2);
    }

    #[test]
    fn accepts_extra_blank_lines() {
        let padded = "\n\n\n1\n00:00:01,000 --> 00:00:02,000\nText\n\n\n\n2\n00:00:03,000 --> 00:00:04,000\nMore\n\n\n";
        let srt = parse_srt(padded).unwrap();
        assert_eq!(srt.events.len(), 2);
    }

    #[test]
    fn rejects_truncated_event() {
        // Index but no timestamp.
        let truncated = "1\n";
        let err = parse_srt(truncated).unwrap_err();
        assert!(err.message.contains("expected timestamp"));
    }

    // --- Serialization round-trip ------------------------------------------

    #[test]
    fn round_trip_preserves_events() {
        let original = parse_srt(SAMPLE_THREE_EVENTS).unwrap();
        let serialized = serialize_srt(&original);
        let reparsed = parse_srt(&serialized).unwrap();
        assert_eq!(original, reparsed);
    }

    #[test]
    fn serialize_renumbers_sequentially() {
        // Even if input had odd indices, output is 1, 2, 3, …
        let weird_indices = "\
42
00:00:01,000 --> 00:00:02,000
A

99
00:00:03,000 --> 00:00:04,000
B
";
        let srt = parse_srt(weird_indices).unwrap();
        let out = serialize_srt(&srt);
        assert!(out.starts_with("1\n"));
        assert!(out.contains("\n2\n"));
    }

    #[test]
    fn empty_input_parses_to_empty_srt() {
        assert_eq!(parse_srt("").unwrap().events.len(), 0);
        assert_eq!(parse_srt("\n\n\n").unwrap().events.len(), 0);
        assert_eq!(serialize_srt(&Srt::default()), "");
    }

    // --- subtract_by_timestamp ---------------------------------------------

    #[test]
    fn subtract_canonical_full_minus_signs() {
        // Full = dialogue + signs. Signs = a subset with identical timing.
        // Result = dialogue only.
        let full = parse_srt("\
1
00:00:01,000 --> 00:00:04,000
Dialogue line 1.

2
00:00:05,000 --> 00:00:06,000
[ Sign: Konbini ]

3
00:00:08,000 --> 00:00:11,000
Dialogue line 2.

4
00:00:12,500 --> 00:00:14,000
[ Sign: Departures ]

5
00:00:16,000 --> 00:00:19,000
Dialogue line 3.
").unwrap();

        let signs = parse_srt("\
1
00:00:05,000 --> 00:00:06,000
[ Sign: Konbini ]

2
00:00:12,500 --> 00:00:14,000
[ Sign: Departures ]
").unwrap();

        let dialogue = subtract_by_timestamp(&full, &signs);
        assert_eq!(dialogue.events.len(), 3);
        assert!(dialogue.events.iter().all(|e| e.text.contains("Dialogue")));
    }

    #[test]
    fn subtract_partial_overlap_does_not_match() {
        // (start=1000, end=4000) vs (start=1000, end=4500) — different end →
        // not a match. Per the design's strict timestamp-pair rule.
        let base = parse_srt("\
1
00:00:01,000 --> 00:00:04,000
Keep me.
").unwrap();
        let subtrahend = parse_srt("\
1
00:00:01,000 --> 00:00:04,500
Different end.
").unwrap();
        let result = subtract_by_timestamp(&base, &subtrahend);
        assert_eq!(result.events.len(), 1);
    }

    #[test]
    fn subtract_with_empty_subtrahend_keeps_everything() {
        let base = parse_srt(SAMPLE_THREE_EVENTS).unwrap();
        let result = subtract_by_timestamp(&base, &Srt::default());
        assert_eq!(result, base);
    }

    #[test]
    fn subtract_with_empty_base_yields_empty() {
        let subtrahend = parse_srt(SAMPLE_THREE_EVENTS).unwrap();
        let result = subtract_by_timestamp(&Srt::default(), &subtrahend);
        assert_eq!(result.events.len(), 0);
    }

    // =========================================================================
    // ASS tests
    // =========================================================================

    fn at(h: u64, m: u64, s: u64, cs: u64) -> AssTime {
        AssTime::from_millis((h * 3600 + m * 60 + s) * 1000 + cs * 10)
    }

    fn format_ass_time(t: AssTime) -> String {
        let total_ms = t.millis;
        let cs = (total_ms % 1000) / 10;
        let total_s = total_ms / 1000;
        let s = total_s % 60;
        let m = (total_s / 60) % 60;
        let h = total_s / 3600;
        format!("{}:{:02}:{:02}.{:02}", h, m, s, cs)
    }

    const SAMPLE_ASS: &str = "\
[Script Info]
Title: Sample
ScriptType: v4.00+
PlayResX: 1920
PlayResY: 1080

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Main,Arial,40,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1
Style: Signs,Arial,30,&H00FFFF00,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,8,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.50,0:00:04.00,Main,,0,0,0,,Hello, world.
Dialogue: 0,0:00:05.00,0:00:06.00,Signs,,0,0,0,,{\\an8}Konbini
Dialogue: 0,0:00:08.00,0:00:11.00,Main,,0,0,0,,Goodbye{\\i1}forever{\\i0}.
Comment: 0,0:00:12.00,0:00:13.00,Main,,0,0,0,,This is a comment
";

    // --- ASS time parsing ---------------------------------------------------

    #[test]
    fn parses_ass_time_centiseconds() {
        assert_eq!(parse_ass_time("0:00:01.50"), Some(AssTime::from_millis(1500)));
        assert_eq!(parse_ass_time("1:23:45.67"), Some(at(1, 23, 45, 67)));
        assert_eq!(parse_ass_time("0:00:00.00"), Some(AssTime::from_millis(0)));
    }

    #[test]
    fn rejects_malformed_ass_time() {
        assert_eq!(parse_ass_time(""), None);
        assert_eq!(parse_ass_time("0:00:01"), None); // no centiseconds
        assert_eq!(parse_ass_time("0:60:00.00"), None); // minutes out of range
        assert_eq!(parse_ass_time("0:00:00.100"), None); // cs out of range
    }

    #[test]
    fn formats_ass_time_uses_single_digit_hour() {
        assert_eq!(format_ass_time(AssTime::from_millis(1500)), "0:00:01.50");
        assert_eq!(format_ass_time(at(1, 23, 45, 67)), "1:23:45.67");
    }

    // --- ASS parsing --------------------------------------------------------

    #[test]
    fn parses_sample_ass_events() {
        let ass = parse_ass(SAMPLE_ASS).expect("parses");
        assert_eq!(ass.events.len(), 4);

        assert_eq!(ass.events[0].kind, "Dialogue");
        assert_eq!(ass.events[0].start, at(0, 0, 1, 50));
        assert_eq!(ass.events[0].end, at(0, 0, 4, 0));
        assert_eq!(ass.events[0].style, "Main");
        assert_eq!(ass.events[0].text, "Hello, world.");

        assert_eq!(ass.events[1].style, "Signs");
        assert_eq!(ass.events[1].text, r"{\an8}Konbini");

        assert_eq!(ass.events[3].kind, "Comment");
        assert_eq!(ass.events[3].style, "Main");
    }

    #[test]
    fn ass_event_with_commas_in_text_preserves_full_text() {
        // The "Hello, world." text contains a comma. Splitting must not
        // truncate at the 10th field.
        let ass = parse_ass(SAMPLE_ASS).unwrap();
        assert_eq!(ass.events[0].text, "Hello, world.");
    }

    #[test]
    fn parses_ass_header_verbatim() {
        let ass = parse_ass(SAMPLE_ASS).unwrap();
        assert!(ass.header.contains("[Script Info]"));
        assert!(ass.header.contains("[V4+ Styles]"));
        assert!(ass.header.contains("Style: Main,Arial,40"));
        assert!(!ass.header.contains("[Events]"));
    }

    #[test]
    fn parses_ass_with_bom_and_crlf() {
        let with_bom_crlf = format!("\u{FEFF}{}", SAMPLE_ASS.replace('\n', "\r\n"));
        let ass = parse_ass(&with_bom_crlf).unwrap();
        assert_eq!(ass.events.len(), 4);
    }

    #[test]
    fn rejects_truncated_ass_event() {
        let bad = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Main\n";
        let err = parse_ass(bad).unwrap_err();
        assert!(err.message.contains("comma-separated fields"), "got: {}", err.message);
    }

    // --- ASS round-trip -----------------------------------------------------

    #[test]
    fn ass_round_trip_preserves_content() {
        let original = parse_ass(SAMPLE_ASS).unwrap();
        let serialized = serialize_ass(&original);
        let reparsed = parse_ass(&serialized).unwrap();
        assert_eq!(original.events, reparsed.events);
    }

    // --- ASS subtract -------------------------------------------------------

    #[test]
    fn ass_subtract_full_minus_signs() {
        let base = parse_ass(SAMPLE_ASS).unwrap();
        // Subtrahend has the timestamp of the Signs event.
        let subtrahend = parse_ass(
            "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
             Dialogue: 0,0:00:05.00,0:00:06.00,Signs,,0,0,0,,Konbini\n",
        )
        .unwrap();
        let result = subtract_ass_by_timestamp(&base, &subtrahend);
        // Originally 4 events (3 Dialogue + 1 Comment); one Dialogue removed.
        assert_eq!(result.events.len(), 3);
        assert!(!result.events.iter().any(|e| e.text.contains("Konbini")));
    }

    // --- ASS filter ---------------------------------------------------------

    fn style_filter(name: &str, retain: bool) -> SubtitleFilter {
        SubtitleFilter {
            style: Some(name.to_string()),
            font: None,
            size: None,
            mode: Some(if retain { FilterMode::Retain } else { FilterMode::Remove }),
        }
    }

    #[test]
    fn ass_filter_retain_keeps_only_matching_style() {
        let base = parse_ass(SAMPLE_ASS).unwrap();
        let result = filter_ass(&base, &style_filter("Main", true));
        // 2 Main Dialogues + 1 Main Comment; Signs Dialogue dropped.
        assert_eq!(result.events.len(), 3);
        assert!(result.events.iter().all(|e| e.style == "Main"));
    }

    #[test]
    fn ass_filter_remove_drops_matching_style() {
        let base = parse_ass(SAMPLE_ASS).unwrap();
        let result = filter_ass(&base, &style_filter("Main", false));
        // Only the Signs event (1 of 4) remains.
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].style, "Signs");
    }

    // --- ASS filter: font / size / combinations + edge cases ---------------

    #[test]
    fn parses_v4plus_styles_into_map() {
        let ass = parse_ass(SAMPLE_ASS).unwrap();
        let main = ass.styles.get("Main").expect("Main style parsed");
        assert_eq!(main.fontname, "Arial");
        assert_eq!(main.fontsize, 40);
        let signs = ass.styles.get("Signs").expect("Signs style parsed");
        assert_eq!(signs.fontname, "Arial");
        assert_eq!(signs.fontsize, 30);
    }

    #[test]
    fn ass_filter_by_font_only() {
        // Build a sample where Main and Signs have different fonts.
        let two_fonts = "[Script Info]
Title: TwoFonts

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Main,Arial,40,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1
Style: Signs,ComicSans,30,&H00FFFF00,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,8,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:02.00,Main,,0,0,0,,Hello
Dialogue: 0,0:00:03.00,0:00:04.00,Signs,,0,0,0,,KONBINI
";
        let ass = parse_ass(two_fonts).unwrap();
        let filter = SubtitleFilter {
            style: None,
            font: Some("ComicSans".to_string()),
            size: None,
            mode: Some(FilterMode::Retain),
        };
        let result = filter_ass(&ass, &filter);
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].style, "Signs");
    }

    #[test]
    fn ass_filter_by_size_only() {
        let ass = parse_ass(SAMPLE_ASS).unwrap();
        // Sample's Main is size 40, Signs is size 30.
        let filter = SubtitleFilter {
            style: None,
            font: None,
            size: Some(30),
            mode: Some(FilterMode::Retain),
        };
        let result = filter_ass(&ass, &filter);
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].style, "Signs");
    }

    #[test]
    fn ass_filter_style_and_font_anded() {
        let ass = parse_ass(SAMPLE_ASS).unwrap();
        // Both Main and Signs use Arial in SAMPLE_ASS, so style=Main + font=Arial
        // should match all Main events (style narrows to Main, font matches Arial).
        let filter = SubtitleFilter {
            style: Some("Main".to_string()),
            font: Some("Arial".to_string()),
            size: None,
            mode: Some(FilterMode::Retain),
        };
        let result = filter_ass(&ass, &filter);
        // 3 Main events (2 Dialogue + 1 Comment).
        assert_eq!(result.events.len(), 3);

        // Same style but wrong font → matches nothing.
        let filter = SubtitleFilter {
            style: Some("Main".to_string()),
            font: Some("ComicSans".to_string()),
            size: None,
            mode: Some(FilterMode::Retain),
        };
        let result = filter_ass(&ass, &filter);
        assert_eq!(result.events.len(), 0);
    }

    #[test]
    fn ass_filter_normalizes_at_prefix_in_font_name() {
        let with_at = "[Script Info]
Title: AtPrefix

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Vertical,@Arial,30,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,8,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:02.00,Vertical,,0,0,0,,vertically written sign
";
        let ass = parse_ass(with_at).unwrap();
        // User writes `font = "Arial"`, expects to match style with `Fontname=@Arial`.
        let filter = SubtitleFilter {
            style: None,
            font: Some("Arial".to_string()),
            size: None,
            mode: Some(FilterMode::Retain),
        };
        let result = filter_ass(&ass, &filter);
        assert_eq!(result.events.len(), 1);
    }

    #[test]
    fn ass_filter_orphan_style_event_not_matched_by_font_filter() {
        // Event references a style that's not in [V4+ Styles] → font/size
        // filters can't resolve, so the event is treated as non-matching.
        let orphan = "[Script Info]
Title: Orphan

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Main,Arial,40,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:02.00,Main,,0,0,0,,Real
Dialogue: 0,0:00:03.00,0:00:04.00,Phantom,,0,0,0,,Orphan
";
        let ass = parse_ass(orphan).unwrap();
        let filter = SubtitleFilter {
            style: None,
            font: Some("Arial".to_string()),
            size: None,
            mode: Some(FilterMode::Retain),
        };
        let result = filter_ass(&ass, &filter);
        // Phantom event excluded since its style isn't in the styles map.
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].style, "Main");
    }

    #[test]
    fn ass_filter_no_styles_section_then_font_filter_matches_nothing() {
        let no_styles = "[Script Info]
Title: NoStyles

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:02.00,Main,,0,0,0,,Hello
";
        let ass = parse_ass(no_styles).unwrap();
        assert!(ass.styles.is_empty());
        let filter = SubtitleFilter {
            style: None,
            font: Some("Arial".to_string()),
            size: None,
            mode: Some(FilterMode::Retain),
        };
        let result = filter_ass(&ass, &filter);
        assert_eq!(result.events.len(), 0);
    }

    #[test]
    fn ass_filter_without_mode_is_no_op() {
        let ass = parse_ass(SAMPLE_ASS).unwrap();
        let filter = SubtitleFilter {
            style: Some("Main".to_string()),
            font: None,
            size: None,
            mode: None,
        };
        let result = filter_ass(&ass, &filter);
        assert_eq!(result.events.len(), ass.events.len());
    }

    #[test]
    fn style_format_honors_column_order() {
        // Same data as SAMPLE_ASS but with Fontname / Fontsize moved to
        // different positions. The parser should still pick them up.
        let reordered = "[Script Info]
Title: Reordered

[V4+ Styles]
Format: Name, Fontsize, Fontname, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Main,40,Arial,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:02.00,Main,,0,0,0,,Hello
";
        let ass = parse_ass(reordered).unwrap();
        let main = ass.styles.get("Main").expect("Main style parsed");
        assert_eq!(main.fontname, "Arial");
        assert_eq!(main.fontsize, 40);
    }

    #[test]
    fn header_round_trip_preserves_styles_section_verbatim() {
        // Even though we parse styles into a map, the verbatim text stays in
        // header so re-emission round-trips.
        let original = parse_ass(SAMPLE_ASS).unwrap();
        let serialized = serialize_ass(&original);
        let reparsed = parse_ass(&serialized).unwrap();
        assert_eq!(original.styles, reparsed.styles);
        assert!(reparsed.header.contains("Style: Main,Arial,40"));
        assert!(reparsed.header.contains("Style: Signs,Arial,30"));
    }

    // --- ASS → SRT ----------------------------------------------------------

    #[test]
    fn ass_to_srt_strips_override_tags() {
        let ass = parse_ass(SAMPLE_ASS).unwrap();
        let srt = ass_to_srt(&ass);
        // 3 Dialogues; Comment excluded.
        assert_eq!(srt.events.len(), 3);
        // Override tags removed; plain text preserved.
        assert_eq!(srt.events[0].text, "Hello, world.");
        assert_eq!(srt.events[1].text, "Konbini");
        assert_eq!(srt.events[2].text, "Goodbyeforever.");
    }

    #[test]
    fn ass_to_srt_preserves_timestamps_in_ms() {
        let ass = parse_ass(SAMPLE_ASS).unwrap();
        let srt = ass_to_srt(&ass);
        // 0:00:01.50 → 1500ms
        assert_eq!(srt.events[0].start, SrtTime::from_millis(1500));
        assert_eq!(srt.events[0].end, SrtTime::from_millis(4000));
    }

    #[test]
    fn ass_to_srt_handles_line_break_escapes() {
        // Synthetic event with \N hard break.
        let ass = parse_ass(
            "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
             Dialogue: 0,0:00:01.00,0:00:02.00,Main,,0,0,0,,first line\\Nsecond line\n",
        )
        .unwrap();
        let srt = ass_to_srt(&ass);
        assert_eq!(srt.events[0].text, "first line\nsecond line");
    }

    #[test]
    fn ass_to_srt_excludes_comments() {
        let ass = parse_ass(SAMPLE_ASS).unwrap();
        let srt = ass_to_srt(&ass);
        assert!(!srt.events.iter().any(|e| e.text.contains("comment")));
    }

    #[test]
    fn ass_to_srt_drops_events_with_only_override_tags() {
        // Event whose text is entirely override tags should be dropped (empty
        // plain text after stripping).
        let ass = parse_ass(
            "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
             Dialogue: 0,0:00:01.00,0:00:02.00,Main,,0,0,0,,{\\fad(500,500)}\nDialogue: 0,0:00:03.00,0:00:04.00,Main,,0,0,0,,Real text\n",
        )
        .unwrap();
        let srt = ass_to_srt(&ass);
        assert_eq!(srt.events.len(), 1);
        assert_eq!(srt.events[0].text, "Real text");
    }
}
