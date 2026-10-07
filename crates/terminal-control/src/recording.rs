use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::frame::{Attributes, Cell, Frame};
use crate::render;
use crate::shot::Shot;
use crate::terminal_core::{SCROLLBACK_ROWS, TerminalCore};

const MAX_VIDEO_FPS: u32 = 1000;
/// Schema version written in the header of every `.termctrl` recording.
pub const FORMAT_VERSION: u8 = 1;

/// One JSON Lines entry in a `.termctrl` recording timeline.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Entry {
    Header {
        version: u8,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    },
    Output {
        at_ms: u64,
        bytes: Vec<u8>,
    },
    Input {
        at_ms: u64,
        origin: InputOrigin,
        bytes: Vec<u8>,
    },
    Resize {
        at_ms: u64,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    },
    Marker {
        at_ms: u64,
        name: String,
    },
}

/// Source of bytes written to the application while recording a session.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputOrigin {
    Client,
    Host,
}

pub struct Writer {
    file: fs::File,
    started: Instant,
    poisoned: bool,
}

impl Writer {
    pub fn new(
        path: &Path,
        started: Instant,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<Self> {
        crate::shot::validate_geometry(rows, cols)?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let mut open = OpenOptions::new();
        open.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open.mode(0o600);
        }
        let mut file = open
            .open(path)
            .with_context(|| format!("create {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .with_context(|| format!("secure {}", path.display()))?;
        }
        serde_json::to_writer(
            &mut file,
            &Entry::Header {
                version: FORMAT_VERSION,
                cols,
                rows,
                cell_width,
                cell_height,
            },
        )
        .context("write recording header")?;
        file.write_all(b"\n").context("write recording newline")?;
        file.flush().context("flush recording header")?;
        Ok(Self {
            file,
            started,
            poisoned: false,
        })
    }

    pub fn output(&mut self, at_ms: u64, bytes: &[u8]) -> Result<()> {
        self.write(Entry::Output {
            at_ms,
            bytes: bytes.to_vec(),
        })
    }

    pub fn output_now(&mut self, bytes: &[u8]) -> Result<()> {
        self.output(self.started.elapsed().as_millis() as u64, bytes)
    }

    pub fn input(&mut self, origin: InputOrigin, bytes: &[u8]) -> Result<()> {
        self.write(Entry::Input {
            at_ms: self.started.elapsed().as_millis() as u64,
            origin,
            bytes: bytes.to_vec(),
        })
    }

    pub fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<()> {
        crate::shot::validate_geometry(rows, cols)?;
        self.write(Entry::Resize {
            at_ms: self.started.elapsed().as_millis() as u64,
            cols,
            rows,
            cell_width,
            cell_height,
        })
    }

    pub fn marker(&mut self, name: &str) -> Result<()> {
        if name.is_empty() {
            bail!("marker name must not be empty");
        }
        self.write(Entry::Marker {
            at_ms: self.started.elapsed().as_millis() as u64,
            name: name.to_owned(),
        })
    }

    fn write(&mut self, entry: Entry) -> Result<()> {
        if self.poisoned {
            bail!("recording writer is unavailable after a failed rollback");
        }
        let mut line = serde_json::to_vec(&entry).context("encode recording event")?;
        line.push(b'\n');
        let offset = self
            .file
            .stream_position()
            .context("checkpoint recording event")?;
        let result = self
            .file
            .write_all(&line)
            .context("write recording event")
            .and_then(|()| self.file.flush().context("flush recording event"));
        if let Err(error) = result {
            if self.file.set_len(offset).is_err()
                || self.file.seek(std::io::SeekFrom::Start(offset)).is_err()
            {
                self.poisoned = true;
                return Err(error).context("recording append failed and could not be rolled back");
            }
            return Err(error);
        }
        Ok(())
    }
}

pub struct VideoOptions {
    pub out: PathBuf,
    pub cell_width: Option<u16>,
    pub cell_height: Option<u16>,
    pub padding: f32,
    pub font_family: String,
    pub pixel_ratio: f32,
    pub hide_cursor: bool,
    pub footer: bool,
    pub fps: u32,
    pub tail: Duration,
    pub include_startup: bool,
    pub edit: Option<PathBuf>,
}

pub fn video(path: &Path, options: &VideoOptions) -> Result<()> {
    if options.fps == 0 {
        bail!("--fps must be greater than zero");
    }
    if options.fps > MAX_VIDEO_FPS {
        bail!("--fps must not exceed {MAX_VIDEO_FPS}");
    }
    let recording = read(path)?;
    let states = states(&recording)?;
    let states = visible_states(&states, options.include_startup);
    if states.is_empty() {
        bail!("recording contains no visible output frames");
    }
    let caption_placement = if options.footer {
        CaptionPlacement::Footer
    } else {
        CaptionPlacement::Inline
    };
    let states = match &options.edit {
        Some(path) => edited_states(
            states,
            &recording.events,
            &read_edit(path)?,
            caption_placement,
        )?,
        None => states.to_vec(),
    };
    let samples = samples(&states, options);
    if let Some(parent) = options
        .out
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let temp = std::env::temp_dir().join(format!(
        "termctrl-video-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&temp).with_context(|| format!("create {}", temp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("secure {}", temp.display()))?;
    }
    let result = render_video_frames(&temp, &recording, &states, &samples, options);
    let _ = fs::remove_dir_all(&temp);
    result
}

/// Parsed recording metadata and timeline entries.
pub struct Recording {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
    pub events: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Marker {
    pub at_ms: u64,
    pub name: String,
}

/// Read and validate a versioned `.termctrl` JSON Lines recording.
pub fn read(path: &Path) -> Result<Recording> {
    let file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut lines = BufReader::new(file).lines();
    let Some(header) = lines.next() else {
        bail!("recording is empty");
    };
    let Entry::Header {
        version,
        cols,
        rows,
        cell_width,
        cell_height,
        ..
    } = serde_json::from_str(&header.context("read recording header")?)
        .context("parse recording header")?
    else {
        bail!("recording does not start with a header");
    };
    if version != FORMAT_VERSION {
        bail!("unsupported recording version {version}");
    }
    crate::shot::validate_geometry(rows, cols)?;
    let events = lines
        .map(|line| {
            serde_json::from_str(&line.context("read recording event")?)
                .context("parse recording event")
        })
        .collect::<Result<Vec<Entry>>>()?;
    if events
        .iter()
        .any(|entry| matches!(entry, Entry::Header { .. }))
    {
        bail!("recording contains a header after the first line");
    }
    Ok(Recording {
        cols,
        rows,
        cell_width,
        cell_height,
        events,
    })
}

pub fn markers(recording: &Recording) -> Vec<Marker> {
    marker_entries(&recording.events).collect()
}

pub fn shot_at(path: &Path, at_ms: Option<u64>, marker: Option<&str>) -> Result<Shot> {
    let recording = read(path)?;
    let at_ms = match (at_ms, marker) {
        (Some(_), Some(_)) => bail!("use --at-ms or --at-marker, not both"),
        (Some(at_ms), None) => at_ms,
        (None, Some(marker)) => *marker_times(&recording.events)?
            .get(marker)
            .with_context(|| format!("recording does not contain marker {marker:?}"))?,
        (None, None) => u64::MAX,
    };
    let replay = replay(&recording, Some(at_ms))?;
    Ok(Shot {
        frame: replay
            .frames
            .last()
            .expect("replay always has an initial frame")
            .frame
            .clone(),
        ansi: replay.ansi,
    })
}

#[derive(Clone)]
struct VideoFrame {
    at_ms: u64,
    frame: Frame,
    footer_caption: Option<String>,
}

struct Replay {
    ansi: Vec<u8>,
    frames: Vec<VideoFrame>,
}

fn states(recording: &Recording) -> Result<Vec<VideoFrame>> {
    Ok(replay(recording, None)?.frames)
}

fn replay(recording: &Recording, cutoff: Option<u64>) -> Result<Replay> {
    let mut terminal = TerminalCore::new(recording.rows, recording.cols, SCROLLBACK_ROWS)?;
    let mut ansi = Vec::new();
    let mut frames: Vec<VideoFrame> = Vec::new();
    frames.push(VideoFrame {
        at_ms: 0,
        frame: terminal.frame()?,
        footer_caption: None,
    });
    for event in &recording.events {
        let at_ms = match event {
            Entry::Output { at_ms, bytes } => {
                if cutoff.is_some_and(|cutoff| *at_ms > cutoff) {
                    continue;
                }
                ansi.extend_from_slice(bytes);
                let _responses = terminal.apply_output(bytes);
                *at_ms
            }
            Entry::Resize {
                at_ms,
                cols,
                rows,
                cell_width,
                cell_height,
            } => {
                if cutoff.is_some_and(|cutoff| *at_ms > cutoff) {
                    continue;
                }
                terminal.resize(*cols, *rows, *cell_width, *cell_height)?;
                *at_ms
            }
            Entry::Input { .. } | Entry::Marker { .. } | Entry::Header { .. } => continue,
        };
        let frame = terminal.frame()?;
        if frames
            .last()
            .is_some_and(|previous| previous.frame == frame)
        {
            continue;
        }
        frames.push(VideoFrame {
            at_ms,
            frame,
            footer_caption: None,
        });
    }
    Ok(Replay { ansi, frames })
}

fn visible_states(states: &[VideoFrame], include_startup: bool) -> &[VideoFrame] {
    if include_startup {
        return states;
    }
    let visible = states
        .iter()
        .position(|frame| has_non_whitespace_text(&frame.frame))
        .or_else(|| {
            states
                .iter()
                .position(|frame| frame.frame.has_visible_content())
        })
        .unwrap_or(states.len());
    &states[visible..]
}

fn has_non_whitespace_text(frame: &Frame) -> bool {
    frame.cells.iter().any(|cell| !cell.text.trim().is_empty())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoEdit {
    clips: Vec<VideoEditClip>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoEditClip {
    from: String,
    to: String,
    caption: Option<String>,
    speed: Option<f64>,
    hold_ms: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CaptionPlacement {
    Inline,
    Footer,
}

fn read_edit(path: &Path) -> Result<VideoEdit> {
    let edit = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("read {}", path.display()))?,
    )
    .with_context(|| format!("parse {}", path.display()))?;
    validate_edit(&edit)?;
    Ok(edit)
}

fn validate_edit(edit: &VideoEdit) -> Result<()> {
    if edit.clips.is_empty() {
        bail!("video edit must contain at least one clip");
    }
    for clip in &edit.clips {
        if clip.from.is_empty() || clip.to.is_empty() {
            bail!("video edit clip markers must not be empty");
        }
        if clip
            .caption
            .as_ref()
            .is_some_and(|caption| caption.chars().count() > 1000)
        {
            bail!("video edit clip caption must not exceed 1000 characters");
        }
    }
    Ok(())
}

fn edited_states(
    states: &[VideoFrame],
    entries: &[Entry],
    edit: &VideoEdit,
    caption_placement: CaptionPlacement,
) -> Result<Vec<VideoFrame>> {
    validate_edit(edit)?;
    let markers = marker_times(entries)?;
    let mut output = Vec::new();
    let mut offset = 0_u64;
    for clip in &edit.clips {
        let from = *markers
            .get(&clip.from)
            .with_context(|| format!("video edit references missing marker {:?}", clip.from))?;
        let to = *markers
            .get(&clip.to)
            .with_context(|| format!("video edit references missing marker {:?}", clip.to))?;
        if from > to {
            bail!("video edit clip {:?} ends before it starts", clip.from);
        }
        let speed = clip.speed.unwrap_or(1.0);
        if !speed.is_finite() || speed <= 0.0 {
            bail!(
                "video edit clip {:?} speed must be greater than zero",
                clip.from
            );
        }
        let clip_start = offset;
        let first = states
            .iter()
            .rfind(|state| state.at_ms <= from)
            .or_else(|| states.first())
            .context("video edit has no visible screen state")?;
        output.push(VideoFrame {
            at_ms: offset,
            frame: frame_with_caption(&first.frame, clip.caption.as_deref(), caption_placement),
            footer_caption: footer_caption(clip.caption.as_deref(), caption_placement),
        });
        output.extend(
            states
                .iter()
                .filter(|state| state.at_ms > from && state.at_ms <= to)
                .map(|state| VideoFrame {
                    at_ms: scale_clip_time(clip_start, from, state.at_ms, speed),
                    frame: frame_with_caption(
                        &state.frame,
                        clip.caption.as_deref(),
                        caption_placement,
                    ),
                    footer_caption: footer_caption(clip.caption.as_deref(), caption_placement),
                }),
        );
        let clip_end = scale_clip_time(clip_start, from, to, speed);
        if output.last().is_some_and(|last| last.at_ms < clip_end)
            && let Some(last) = output.last()
        {
            output.push(VideoFrame {
                at_ms: clip_end,
                frame: last.frame.clone(),
                footer_caption: last.footer_caption.clone(),
            });
        }
        let hold_ms = clip.hold_ms.unwrap_or(0);
        offset = clip_end.saturating_add(hold_ms);
        if hold_ms > 0
            && let Some(last) = output.last()
        {
            output.push(VideoFrame {
                at_ms: offset,
                frame: last.frame.clone(),
                footer_caption: last.footer_caption.clone(),
            });
        }
    }
    Ok(output)
}

fn frame_with_caption(frame: &Frame, caption: Option<&str>, placement: CaptionPlacement) -> Frame {
    match placement {
        CaptionPlacement::Inline => annotate(frame.clone(), caption),
        CaptionPlacement::Footer => frame.clone(),
    }
}

fn footer_caption(caption: Option<&str>, placement: CaptionPlacement) -> Option<String> {
    (placement == CaptionPlacement::Footer)
        .then(|| caption.map(str::to_owned))
        .flatten()
}

fn scale_clip_time(clip_start: u64, from: u64, at_ms: u64, speed: f64) -> u64 {
    clip_start + ((at_ms.saturating_sub(from) as f64) / speed) as u64
}

fn marker_times(entries: &[Entry]) -> Result<HashMap<String, u64>> {
    let mut markers = HashMap::new();
    for marker in marker_entries(entries) {
        if markers.insert(marker.name.clone(), marker.at_ms).is_some() {
            bail!("recording contains duplicate marker {:?}", marker.name);
        }
    }
    Ok(markers)
}

fn marker_entries(entries: &[Entry]) -> impl Iterator<Item = Marker> + '_ {
    entries.iter().filter_map(|entry| match entry {
        Entry::Marker { at_ms, name } => Some(Marker {
            at_ms: *at_ms,
            name: name.clone(),
        }),
        _ => None,
    })
}

fn annotate(mut frame: Frame, caption: Option<&str>) -> Frame {
    let Some(caption) = caption else {
        return frame;
    };
    let text: String = ['>', ' ']
        .into_iter()
        .chain(caption.chars())
        .take(usize::from(frame.cols.saturating_sub(2)))
        .collect();
    if text.is_empty() {
        return frame;
    }
    let y = frame.rows;
    frame.rows = frame.rows.saturating_add(2);
    push_text_cell(
        &mut frame,
        1,
        y,
        text,
        u16::MAX,
        Attributes {
            bold: true,
            ..Attributes::default()
        },
    );
    frame
}

fn samples(states: &[VideoFrame], options: &VideoOptions) -> Vec<usize> {
    if states.is_empty() {
        return Vec::new();
    }
    let mut timeline = Vec::with_capacity(states.len());
    let mut at_ms = 0_u64;
    for index in 0..states.len() {
        timeline.push(at_ms);
        if let Some(next) = states.get(index + 1) {
            at_ms = at_ms.saturating_add(next.at_ms.saturating_sub(states[index].at_ms));
        }
    }
    let end_ms = at_ms.saturating_add(options.tail.as_millis() as u64);
    let mut output = Vec::new();
    let mut state = 0;
    let mut sample = 0_u64;
    loop {
        let sample_ms = u128::from(sample) * 1000 / u128::from(options.fps);
        if sample_ms > u128::from(end_ms) {
            break;
        }
        let sample_ms = sample_ms as u64;
        while state + 1 < timeline.len() && timeline[state + 1] <= sample_ms {
            state += 1;
        }
        output.push(state);
        sample += 1;
    }
    if output.last() != Some(&(states.len() - 1)) {
        output.push(states.len() - 1);
    }
    output
}

fn render_video_frames(
    temp: &Path,
    recording: &Recording,
    states: &[VideoFrame],
    samples: &[usize],
    options: &VideoOptions,
) -> Result<()> {
    eprintln!("Rendering {} sampled frames...", samples.len());
    let cols = states
        .iter()
        .map(|state| state.frame.cols)
        .max()
        .unwrap_or(recording.cols);
    let rows = states
        .iter()
        .map(|state| state.frame.rows)
        .max()
        .unwrap_or(recording.rows);
    let base_keys = states
        .iter()
        .map(|state| render_key(&state.frame, cols, rows, options.hide_cursor))
        .collect::<Vec<_>>();
    let mut rendered = HashMap::<Frame, PathBuf>::new();
    let renderer = render::PngRenderer::new();
    let render_options = render::Options {
        cell_width: f32::from(options.cell_width.unwrap_or(recording.cell_width)),
        cell_height: f32::from(options.cell_height.unwrap_or(recording.cell_height)),
        font_size: f32::from(options.cell_height.unwrap_or(recording.cell_height)) * 0.78,
        padding: options.padding,
        font_family: options.font_family.clone(),
        show_cursor: !options.hide_cursor,
    };
    for (index, state) in samples.iter().enumerate() {
        let path = temp.join(format!("frame-{index:06}.png"));
        if options.footer {
            let key = with_footer(
                base_keys[*state].clone(),
                states[*state].footer_caption.as_deref(),
                (u128::from(index as u64) * 1000 / u128::from(options.fps)) as u64,
            );
            render_or_link(
                &renderer,
                &mut rendered,
                &key,
                &path,
                &render_options,
                options.pixel_ratio,
            )?;
        } else {
            render_or_link(
                &renderer,
                &mut rendered,
                &base_keys[*state],
                &path,
                &render_options,
                options.pixel_ratio,
            )?;
        }
    }
    eprintln!("Rendered {} unique screens.", rendered.len());
    eprintln!("Encoding {}...", options.out.display());
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-framerate"])
        .arg(options.fps.to_string())
        .arg("-i")
        .arg(temp.join("frame-%06d.png"))
        .args(["-vf", "format=yuv420p", "-movflags", "+faststart"])
        .arg(&options.out)
        .status()
        .context("run ffmpeg; install ffmpeg to export recorded sessions as video")?;
    if !status.success() {
        bail!("ffmpeg failed while exporting {}", options.out.display());
    }
    Ok(())
}

fn render_or_link(
    renderer: &render::PngRenderer,
    rendered: &mut HashMap<Frame, PathBuf>,
    key: &Frame,
    path: &Path,
    options: &render::Options,
    pixel_ratio: f32,
) -> Result<()> {
    if let Some(existing) = rendered.get(key) {
        fs::hard_link(existing, path).or_else(|_| fs::copy(existing, path).map(|_| ()))?;
        return Ok(());
    }
    renderer.render(&render::svg(key, options), path, pixel_ratio)?;
    rendered.insert(key.clone(), path.to_path_buf());
    Ok(())
}

fn render_key(frame: &Frame, cols: u16, rows: u16, hide_cursor: bool) -> Frame {
    let mut frame = frame.clone();
    frame.cols = cols;
    frame.rows = rows;
    if hide_cursor {
        frame.cursor = None;
    }
    frame
}

fn with_footer(mut frame: Frame, caption: Option<&str>, elapsed_ms: u64) -> Frame {
    const BRAND: &str = "TERMINAL CONTROL";
    let footer_y = frame.rows.saturating_add(1);
    frame.rows = frame.rows.saturating_add(2);

    let timecode = format_timecode(elapsed_ms);
    let brand_width = text_width(BRAND);
    let time_width = text_width(&timecode);
    let brand = (brand_width <= frame.cols).then(|| (frame.cols - brand_width, BRAND));
    let time = brand.and_then(|(brand_x, _)| {
        let x = brand_x.checked_sub(time_width.saturating_add(1))?;
        Some((x, timecode.as_str()))
    });
    let reserved_from = time
        .map(|(x, _)| x.saturating_sub(1))
        .or_else(|| brand.map(|(x, _)| x.saturating_sub(1)))
        .unwrap_or(frame.cols);

    if let Some(caption) = caption {
        push_footer_cell(
            &mut frame,
            1,
            footer_y,
            caption,
            reserved_from.saturating_sub(1),
            true,
            false,
        );
    }
    if let Some((x, text)) = time {
        push_footer_cell(&mut frame, x, footer_y, text, time_width, true, false);
    }
    if let Some((x, text)) = brand {
        push_footer_cell(&mut frame, x, footer_y, text, brand_width, false, true);
    }
    frame
}

fn push_footer_cell(
    frame: &mut Frame,
    x: u16,
    y: u16,
    text: &str,
    max_width: u16,
    faint: bool,
    bold: bool,
) {
    push_text_cell(
        frame,
        x,
        y,
        text,
        max_width,
        Attributes {
            bold,
            faint,
            ..Attributes::default()
        },
    );
}

fn push_text_cell(
    frame: &mut Frame,
    x: u16,
    y: u16,
    text: impl AsRef<str>,
    max_width: u16,
    attributes: Attributes,
) {
    if x >= frame.cols || y >= frame.rows || max_width == 0 {
        return;
    }
    let available = (frame.cols - x).min(max_width);
    let text = truncate(text.as_ref(), available);
    if text.is_empty() {
        return;
    }
    frame.cells.push(Cell {
        x,
        y,
        width: text_width(&text),
        text,
        foreground: frame.foreground,
        background: frame.background,
        attributes,
    });
}

fn truncate(text: &str, max_width: u16) -> String {
    text.chars().take(usize::from(max_width)).collect()
}

fn text_width(text: &str) -> u16 {
    text.chars().count().min(usize::from(u16::MAX)) as u16
}

fn format_timecode(elapsed_ms: u64) -> String {
    let total_seconds = elapsed_ms / 1000;
    let seconds = total_seconds % 60;
    let minutes = (total_seconds / 60) % 60;
    let hours = total_seconds / 3600;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(text: &str) -> Frame {
        Frame {
            version: 1,
            cols: 40,
            rows: 1,
            foreground: crate::frame::DEFAULT_FOREGROUND,
            background: crate::frame::DEFAULT_BACKGROUND,
            cursor: None,
            cells: (!text.is_empty())
                .then(|| crate::frame::Cell {
                    x: 0,
                    y: 0,
                    text: text.to_owned(),
                    width: 1,
                    foreground: crate::frame::DEFAULT_FOREGROUND,
                    background: crate::frame::DEFAULT_BACKGROUND,
                    attributes: crate::frame::Attributes::default(),
                })
                .into_iter()
                .collect(),
        }
    }

    fn options() -> VideoOptions {
        VideoOptions {
            out: PathBuf::from("video.mp4"),
            cell_width: None,
            cell_height: None,
            padding: 0.0,
            font_family: String::new(),
            pixel_ratio: 1.0,
            hide_cursor: true,
            footer: false,
            fps: 20,
            tail: Duration::ZERO,
            include_startup: false,
            edit: None,
        }
    }

    fn edit(from: &str, to: &str) -> VideoEdit {
        VideoEdit {
            clips: vec![VideoEditClip {
                from: from.to_owned(),
                to: to.to_owned(),
                caption: None,
                speed: None,
                hold_ms: None,
            }],
        }
    }

    fn painted_frame() -> Frame {
        let mut terminal = TerminalCore::new(1, 2, 0).unwrap();
        let _responses = terminal.apply_output(b"\x1b[48;2;30;34;42m ");
        terminal.frame().unwrap()
    }

    #[test]
    fn realtime_sampling_preserves_recorded_duration() {
        let initial = frame("a");
        let final_frame = frame("b");

        let frames = samples(
            &[
                VideoFrame {
                    at_ms: 0,
                    frame: initial,
                    footer_caption: None,
                },
                VideoFrame {
                    at_ms: 4000,
                    frame: final_frame.clone(),
                    footer_caption: None,
                },
            ],
            &options(),
        );

        assert_eq!(frames.len(), 81);
        assert_eq!(frames.last(), Some(&1));
    }

    #[test]
    fn edit_plan_stitches_marker_ranges_with_speed_hold_and_caption() {
        let first = frame("a");
        let second = frame("b");
        let states = edited_states(
            &[
                VideoFrame {
                    at_ms: 0,
                    frame: first.clone(),
                    footer_caption: None,
                },
                VideoFrame {
                    at_ms: 1000,
                    frame: second.clone(),
                    footer_caption: None,
                },
                VideoFrame {
                    at_ms: 2000,
                    frame: first.clone(),
                    footer_caption: None,
                },
            ],
            &[
                Entry::Marker {
                    at_ms: 0,
                    name: "start".to_owned(),
                },
                Entry::Marker {
                    at_ms: 2000,
                    name: "done".to_owned(),
                },
            ],
            &VideoEdit {
                clips: vec![VideoEditClip {
                    from: "start".to_owned(),
                    to: "done".to_owned(),
                    caption: Some("accelerated".to_owned()),
                    speed: Some(2.0),
                    hold_ms: Some(500),
                }],
            },
            CaptionPlacement::Inline,
        )
        .unwrap();

        assert_eq!(
            states.iter().map(|state| state.at_ms).collect::<Vec<_>>(),
            [0, 500, 1000, 1500]
        );
        assert_eq!(states[0].frame.rows, 3);
        assert!(states[0].frame.text().contains("accelerated"));
        assert_eq!(states.last().unwrap().frame.text(), states[2].frame.text());
    }

    #[test]
    fn edit_plan_preserves_quiet_time_before_clip_end() {
        let states = edited_states(
            &[
                VideoFrame {
                    at_ms: 0,
                    frame: frame("a"),
                    footer_caption: None,
                },
                VideoFrame {
                    at_ms: 1000,
                    frame: frame("b"),
                    footer_caption: None,
                },
            ],
            &[
                Entry::Marker {
                    at_ms: 0,
                    name: "start".to_owned(),
                },
                Entry::Marker {
                    at_ms: 2000,
                    name: "done".to_owned(),
                },
            ],
            &edit("start", "done"),
            CaptionPlacement::Inline,
        )
        .unwrap();

        assert_eq!(
            states.iter().map(|state| state.at_ms).collect::<Vec<_>>(),
            [0, 1000, 2000]
        );
        assert_eq!(states[1].frame, states[2].frame);
    }

    #[test]
    fn edit_plan_can_place_captions_in_footer_metadata() {
        let states = edited_states(
            &[VideoFrame {
                at_ms: 0,
                frame: frame("a"),
                footer_caption: None,
            }],
            &[
                Entry::Marker {
                    at_ms: 0,
                    name: "start".to_owned(),
                },
                Entry::Marker {
                    at_ms: 0,
                    name: "done".to_owned(),
                },
            ],
            &VideoEdit {
                clips: vec![VideoEditClip {
                    from: "start".to_owned(),
                    to: "done".to_owned(),
                    caption: Some("footer caption".to_owned()),
                    speed: None,
                    hold_ms: None,
                }],
            },
            CaptionPlacement::Footer,
        )
        .unwrap();

        assert_eq!(states[0].frame.rows, 1);
        assert_eq!(states[0].frame.text(), "a");
        assert_eq!(states[0].footer_caption.as_deref(), Some("footer caption"));
    }

    #[test]
    fn footer_adds_caption_timecode_and_branding() {
        let frame = with_footer(frame("body"), Some("demo caption"), 65_000);
        let text = frame.text();
        let time = frame
            .cells
            .iter()
            .find(|cell| cell.text == "01:05")
            .unwrap();
        let brand = frame
            .cells
            .iter()
            .find(|cell| cell.text == "TERMINAL CONTROL")
            .unwrap();

        assert_eq!(frame.rows, 3);
        assert!(text.contains("body"));
        assert!(text.contains("demo caption"));
        assert_eq!(time.x + time.width + 1, brand.x);
        assert!(time.attributes.faint);
        assert!(brand.attributes.bold);
    }

    #[test]
    fn footer_avoids_overlapping_cells_when_narrow() {
        let mut narrow = frame("body");
        narrow.cols = 10;
        let frame = with_footer(narrow, Some("demo caption"), 65_000);
        let mut spans = frame
            .cells
            .iter()
            .filter(|cell| cell.y == 2)
            .map(|cell| (cell.x, cell.x + cell.width))
            .collect::<Vec<_>>();
        spans.sort();

        assert!(spans.iter().all(|(_, end)| *end <= frame.cols));
        assert!(spans.windows(2).all(|pair| pair[0].1 <= pair[1].0));
    }

    #[test]
    fn edit_plan_rejects_missing_or_duplicate_markers() {
        let states = [VideoFrame {
            at_ms: 0,
            frame: frame("a"),
            footer_caption: None,
        }];

        assert!(
            edited_states(
                &states,
                &[],
                &edit("missing", "done"),
                CaptionPlacement::Inline
            )
            .is_err()
        );
        assert!(
            edited_states(
                &states,
                &[
                    Entry::Marker {
                        at_ms: 0,
                        name: "start".to_owned(),
                    },
                    Entry::Marker {
                        at_ms: 1,
                        name: "start".to_owned(),
                    },
                ],
                &edit("start", "start"),
                CaptionPlacement::Inline,
            )
            .is_err()
        );
    }

    #[test]
    fn preserves_input_origin_and_binary_output() {
        let temp =
            std::env::temp_dir().join(format!("termctrl-recording-test-{}", std::process::id()));
        let mut writer = Writer::new(&temp, Instant::now(), 2, 1, 9, 18).unwrap();
        writer.output(1, &[0, 255, b'A']).unwrap();
        writer.input(InputOrigin::Host, b"reply").unwrap();
        writer.marker("checkpoint").unwrap();
        drop(writer);

        let recording = read(&temp).unwrap();
        let _ = fs::remove_file(temp);
        assert!(matches!(
            &recording.events[0],
            Entry::Output { at_ms: 1, bytes } if bytes == &[0, 255, b'A']
        ));
        assert!(matches!(
            &recording.events[1],
            Entry::Input { origin: InputOrigin::Host, bytes, .. } if bytes == b"reply"
        ));
        assert!(matches!(
            &recording.events[2],
            Entry::Marker { name, .. } if name == "checkpoint"
        ));
    }

    #[test]
    fn replays_resized_recordings_on_a_stable_video_canvas() {
        let recording = Recording {
            cols: 2,
            rows: 1,
            cell_width: 9,
            cell_height: 18,
            events: vec![
                Entry::Output {
                    at_ms: 1,
                    bytes: b"a".to_vec(),
                },
                Entry::Resize {
                    at_ms: 2,
                    cols: 4,
                    rows: 2,
                    cell_width: 9,
                    cell_height: 18,
                },
            ],
        };

        let states = states(&recording).unwrap();
        let cols = states.iter().map(|state| state.frame.cols).max().unwrap();
        let rows = states.iter().map(|state| state.frame.rows).max().unwrap();
        let frames = states
            .iter()
            .map(|state| render_key(&state.frame, cols, rows, true))
            .collect::<Vec<_>>();

        assert!(
            frames
                .iter()
                .all(|frame| (frame.cols, frame.rows) == (4, 2))
        );
        assert_eq!(frames.last().unwrap().text(), "a");
    }

    #[test]
    fn replay_preserves_reflowed_scrollback_across_multiple_resizes() {
        let recording = Recording {
            cols: 10,
            rows: 3,
            cell_width: 9,
            cell_height: 18,
            events: vec![
                Entry::Output {
                    at_ms: 1,
                    bytes: b"abcdefghijklmnopqrst".to_vec(),
                },
                Entry::Resize {
                    at_ms: 2,
                    cols: 5,
                    rows: 3,
                    cell_width: 9,
                    cell_height: 18,
                },
                Entry::Resize {
                    at_ms: 3,
                    cols: 10,
                    rows: 3,
                    cell_width: 9,
                    cell_height: 18,
                },
            ],
        };

        let states = states(&recording).unwrap();

        assert_eq!(
            states.last().unwrap().frame.text(),
            "abcdefghij\nklmnopqrst"
        );
    }

    #[test]
    fn preserves_background_only_output_when_no_text_is_recorded() {
        let painted = painted_frame();
        let frames = vec![
            VideoFrame {
                at_ms: 0,
                frame: frame(""),
                footer_caption: None,
            },
            VideoFrame {
                at_ms: 1,
                frame: painted.clone(),
                footer_caption: None,
            },
        ];

        assert_eq!(visible_states(&frames, false)[0].frame, painted);
    }

    #[test]
    fn keeps_final_change_between_sampling_ticks() {
        let initial = frame("a");
        let final_frame = frame("b");
        let frames = samples(
            &[
                VideoFrame {
                    at_ms: 0,
                    frame: initial.clone(),
                    footer_caption: None,
                },
                VideoFrame {
                    at_ms: 1,
                    frame: final_frame.clone(),
                    footer_caption: None,
                },
            ],
            &options(),
        );

        assert_eq!(frames, vec![0, 1]);
    }

    #[test]
    fn samples_fractional_frame_intervals_without_an_early_transition() {
        let initial = frame("a");
        let final_frame = frame("b");
        let mut options = options();
        options.fps = 30;

        let frames = samples(
            &[
                VideoFrame {
                    at_ms: 0,
                    frame: initial.clone(),
                    footer_caption: None,
                },
                VideoFrame {
                    at_ms: 100,
                    frame: final_frame.clone(),
                    footer_caption: None,
                },
            ],
            &options,
        );

        assert_eq!(frames, vec![0, 0, 0, 1]);
    }

    #[test]
    fn rejects_excessive_video_frame_rates_before_reading_input() {
        let mut options = options();
        options.fps = MAX_VIDEO_FPS + 1;

        assert_eq!(
            video(Path::new("not-read.termctrl"), &options)
                .unwrap_err()
                .to_string(),
            "--fps must not exceed 1000"
        );
    }

    #[test]
    fn rejects_invalid_geometry_and_repeated_headers() {
        let invalid =
            std::env::temp_dir().join(format!("termctrl-invalid-recording-{}", std::process::id()));
        fs::write(&invalid, "{\"type\":\"header\",\"version\":1,\"cols\":0,\"rows\":1,\"cell_width\":9,\"cell_height\":18}\n").unwrap();
        assert!(read(&invalid).is_err());
        fs::write(&invalid, "{\"type\":\"header\",\"version\":1,\"cols\":1,\"rows\":1,\"cell_width\":9,\"cell_height\":18}\n{\"type\":\"header\",\"version\":1,\"cols\":1,\"rows\":1,\"cell_width\":9,\"cell_height\":18}\n").unwrap();
        assert!(read(&invalid).is_err());
        let _ = fs::remove_file(invalid);
    }
}
