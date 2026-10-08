//! Local Mermaid rendering. One lazy worker owns CPU-heavy layout/raster work;
//! the UI requests immutable artifacts and never waits for the renderer.
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU32, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

use crossterm::style::Color;
use merman::{
    ascii::{AsciiOutputOutcome, AsciiRenderOptions, AsciiResourceLimitId, AsciiResourcePolicy},
    resources::{InputResourceLimitId, InputResourcePolicy, ResourceProfile},
    svg::{
        export::{RasterFitBox, RasterOptions},
        RenderResourcePolicy, RootBackgroundPostprocessor, SvgPipeline,
    },
    AsciiRequest, Engine, MermaidConfig, OperationControl, ParseOptions, PngRequest, RenderOutput,
    RenderRequest, Renderer, SvgEnvironment, SvgRequest,
};

const MAX_SOURCE: usize = 64 * 1024;
const MAX_CACHE: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 64;
const DEBOUNCE: Duration = Duration::from_millis(150);
static NEXT_IMAGE: AtomicU32 = AtomicU32::new(1);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Auto,
    Unicode,
    Kitty,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Palette {
    pub light: bool,
    pub bg: [u8; 3],
    pub fg: [u8; 3],
    pub accent: [u8; 3],
}

impl Palette {
    pub fn for_theme(theme: crate::theme::Theme) -> Self {
        let rgb = |c| match c {
            Color::Rgb { r, g, b } => Some([r, g, b]),
            _ => None,
        };
        let bg = rgb(theme.bg).unwrap_or([15, 23, 42]);
        let light =
            u32::from(bg[0]) * 299 + u32::from(bg[1]) * 587 + u32::from(bg[2]) * 114 > 150_000;
        Self {
            light,
            bg,
            fg: rgb(theme.fg).unwrap_or([226, 232, 240]),
            accent: rgb(theme.accent).unwrap_or(if light { [3, 105, 161] } else { [56, 189, 248] }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub source: Arc<str>,
    pub palette: Palette,
    pub graphical: bool,
}

pub enum Artifact {
    Text(Vec<String>),
    Image {
        id: u32,
        png: Vec<u8>,
        width: u32,
        height: u32,
    },
    Error(String),
}

impl Artifact {
    fn bytes(&self) -> usize {
        match self {
            Self::Text(rows) => rows.iter().map(String::len).sum(),
            Self::Image { png, .. } => png.len(),
            Self::Error(s) => s.len(),
        }
    }
}

struct Entry {
    queued: Instant,
    used: u64,
    result: Option<Arc<Artifact>>,
}

struct Worker {
    tx: mpsc::SyncSender<(Key, OperationControl)>,
    rx: mpsc::Receiver<(Key, Arc<Artifact>)>,
}

#[derive(Default)]
pub struct Service {
    cache: HashMap<Key, Entry>,
    demand: HashSet<Key>,
    worker: Option<Worker>,
    active: Option<(Key, OperationControl)>,
    pub revision: u64,
    frame: u64,
}

impl Service {
    pub fn has_demand(&self) -> bool {
        !self.demand.is_empty()
    }
    pub fn begin_frame(&mut self) {
        self.demand.clear();
        self.frame += 1;
    }

    pub fn touch(&mut self, key: &Key) {
        self.demand.insert(key.clone());
        if let Some(entry) = self.cache.get_mut(key) {
            entry.used = self.frame;
        }
    }

    pub fn request(&mut self, key: &Key) -> Option<Arc<Artifact>> {
        self.touch(key);
        if key.source.len() > MAX_SOURCE {
            return Some(Arc::new(Artifact::Error(
                "diagram exceeds 64 KiB source limit".into(),
            )));
        }
        if !self.cache.contains_key(key) {
            while self.cache.len() >= MAX_ENTRIES {
                if !self.evict_one() {
                    return Some(Arc::new(Artifact::Error(
                        "too many diagrams in preview (limit 64)".into(),
                    )));
                }
            }
            self.cache.insert(
                key.clone(),
                Entry {
                    queued: Instant::now(),
                    used: self.frame,
                    result: None,
                },
            );
        }
        self.cache.get(key).and_then(|entry| entry.result.clone())
    }

    pub fn end_frame(&mut self) {
        if let Some((key, control)) = &self.active {
            if !self.demand.contains(key) {
                control.cancel();
            }
        }
        let previous = self.cache.len();
        self.cache
            .retain(|key, entry| entry.result.is_some() || self.demand.contains(key));
        if self.cache.len() != previous {
            // Saved pane layouts may still contain a pending marker. They
            // must refresh and re-request it when that pane is reopened.
            self.revision += 1;
        }
    }

    fn evict_one(&mut self) -> bool {
        let victim = self
            .cache
            .iter()
            .filter(|(key, _)| {
                !self.demand.contains(*key)
                    && !self
                        .active
                        .as_ref()
                        .is_some_and(|(active, _)| active == *key)
            })
            .min_by_key(|(_, entry)| entry.used)
            .map(|(key, _)| key.clone());
        if let Some(key) = victim {
            self.cache.remove(&key);
            self.revision += 1;
            true
        } else {
            false
        }
    }

    pub fn image(&self, id: u32) -> Option<Arc<Artifact>> {
        self.cache.values().filter_map(|entry| entry.result.as_ref())
            .find(|artifact| matches!(artifact.as_ref(), Artifact::Image { id: image, .. } if *image == id))
            .cloned()
    }

    /// Called during active frames and idle polling, so results also repaint
    /// when there is no keyboard input. A single worker bounds concurrency.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let replies: Vec<_> = self
            .worker
            .as_ref()
            .map(|w| w.rx.try_iter().collect())
            .unwrap_or_default();
        for (key, mut artifact) in replies {
            let cancelled = self
                .active
                .take()
                .is_some_and(|(_, control)| control.is_cancelled());
            if cancelled {
                // Reopening can reinsert the same content key before its
                // cancelled worker replies. That reply belongs to the old
                // request and must not poison the new pending cache entry.
                continue;
            }
            while self
                .cache
                .values()
                .map(|e| e.result.as_ref().map_or(0, |a| a.bytes()))
                .sum::<usize>()
                + artifact.bytes()
                > MAX_CACHE
            {
                if !self.evict_one() {
                    artifact = Arc::new(Artifact::Error(
                        "preview diagram cache exceeds 32 MiB budget".into(),
                    ));
                    break;
                }
            }
            if let Some(entry) = self.cache.get_mut(&key) {
                entry.result = Some(artifact);
                self.revision += 1;
                changed = true;
            }
        }
        if self.active.is_some() {
            return changed;
        }
        let next = self
            .cache
            .iter()
            .filter(|(key, entry)| {
                entry.result.is_none()
                    && self.demand.contains(*key)
                    && entry.queued.elapsed() >= DEBOUNCE
            })
            .min_by_key(|(_, entry)| entry.queued)
            .map(|(key, _)| key.clone());
        if let Some(key) = next {
            let worker = self.worker.get_or_insert_with(|| {
                let (tx, jobs) = mpsc::sync_channel::<(Key, OperationControl)>(1);
                let (results, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    while let Ok((key, control)) = jobs.recv() {
                        let artifact = Arc::new(render(&key, control));
                        if results.send((key, artifact)).is_err() {
                            break;
                        }
                    }
                });
                Worker { tx, rx }
            });
            let control = OperationControl::new().with_deadline(Duration::from_secs(3));
            if worker.tx.try_send((key.clone(), control.clone())).is_ok() {
                self.active = Some((key, control));
            }
        }
        changed
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if let Some((_, control)) = &self.active {
            control.cancel();
        }
    }
}

pub fn clean_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                '\u{fffd}'
            } else {
                c
            }
        })
        .collect()
}

fn render(key: &Key, control: OperationControl) -> Artifact {
    match render_result(key, control) {
        Ok(artifact) => artifact,
        Err(error) => Artifact::Error(clean_text(&error).chars().take(240).collect()),
    }
}

fn render_result(key: &Key, control: OperationControl) -> Result<Artifact, String> {
    let hex = |rgb: [u8; 3]| format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
    let p = key.palette;
    let site = MermaidConfig::from_value(serde_json::json!({
        "theme": "base", "fontFamily": "DejaVu Sans", "fontSize": 16,
        "securityLevel": "strict", "htmlLabels": false,
        "secure": ["securityLevel", "secure", "htmlLabels"],
        "flowchart": {"htmlLabels": false, "curve": "basis", "nodeSpacing": 40, "rankSpacing": 55},
        "themeVariables": {
            "background": hex(p.bg), "primaryTextColor": hex(p.fg),
            "primaryColor": if p.light { "#e0f2fe" } else { "#1e354e" },
            "primaryBorderColor": hex(p.accent),
            "secondaryColor": if p.light { "#ede9fe" } else { "#312e55" },
            "tertiaryColor": if p.light { "#dcfce7" } else { "#164239" },
            "lineColor": if p.light { "#64748b" } else { "#94a3b8" },
            "textColor": hex(p.fg), "nodeTextColor": hex(p.fg),
            "edgeLabelBackground": hex(p.bg), "clusterBkg": hex(p.bg),
            "clusterBorder": if p.light { "#94a3b8" } else { "#475569" },
            "actorTextColor": hex(p.fg), "actorBkg": if p.light { "#e0f2fe" } else { "#1e354e" },
            "actorBorder": hex(p.accent), "signalColor": hex(p.fg), "signalTextColor": hex(p.fg),
            "rowOdd": if p.light { "#f1f5f9" } else { "#22384d" },
            "rowEven": if p.light { "#e0f2fe" } else { "#1c2e42" },
            "attributeBackgroundColorOdd": if p.light { "#f1f5f9" } else { "#22384d" },
            "attributeBackgroundColorEven": if p.light { "#e0f2fe" } else { "#1c2e42" },
            "pieStrokeColor": hex(p.bg), "pieOuterStrokeColor": hex(p.accent)
        }
    }));
    let input = InputResourcePolicy::for_profile(ResourceProfile::Constrained)
        .with_limit(InputResourceLimitId::MaxSourceBytes, MAX_SOURCE)
        .map_err(|e| e.to_string())?
        .with_limit(InputResourceLimitId::MaxModelItems, 800)
        .map_err(|e| e.to_string())?;
    let renderer = Renderer::new()
        .with_engine(Engine::new().with_site_config(site))
        .with_parse_options(ParseOptions::strict())
        .with_resource_policy(input);
    if key.graphical {
        let svg = SvgRequest {
            pipeline: Some(
                SvgPipeline::resvg_safe()
                    .with_postprocessor(RootBackgroundPostprocessor::new(hex(p.bg))),
            ),
            environment: SvgEnvironment::deterministic()
                .with_resource_policy(RenderResourcePolicy::constrained()),
            ..Default::default()
        };
        let request = RenderRequest::png(
            &key.source,
            control,
            PngRequest {
                svg,
                options: RasterOptions::default()
                    .with_fit_to(RasterFitBox::contain(1600, 2200))
                    .with_scale(1.5)
                    .with_background(hex(p.bg)),
            },
        );
        match renderer.render(request).map_err(|e| e.to_string())? {
            RenderOutput::Png(Some(output)) => {
                if output.bytes.len() > MAX_CACHE / 2 {
                    return Err("rendered diagram exceeds image budget".into());
                }
                let id = NEXT_IMAGE.fetch_add(1, Ordering::Relaxed);
                if id > 0xff_ffff {
                    return Err("terminal image IDs exhausted".into());
                }
                Ok(Artifact::Image {
                    id,
                    png: output.bytes,
                    width: output.plan.width_px,
                    height: output.plan.height_px,
                })
            }
            _ => Err("no supported Mermaid diagram detected".into()),
        }
    } else {
        let resources = AsciiResourcePolicy::default()
            .with_limit(AsciiResourceLimitId::MaxOutputBytes, 1024 * 1024)
            .map_err(|e| e.to_string())?;
        match renderer
            .render(RenderRequest::ascii(
                &key.source,
                control,
                AsciiRequest {
                    options: AsciiRenderOptions::unicode(),
                    resources,
                    ..Default::default()
                },
            ))
            .map_err(|e| e.to_string())?
        {
            RenderOutput::Ascii(Some(report)) => match report.outcome {
                AsciiOutputOutcome::Primary | AsciiOutputOutcome::WideAllowed => {
                    Ok(Artifact::Text(
                        clean_text(&report.text)
                            .lines()
                            .map(str::to_string)
                            .collect(),
                    ))
                }
                _ => Err("diagram has no complete Unicode layout; use graphics mode".into()),
            },
            _ => Err("no supported Mermaid diagram detected".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(source: &str, graphical: bool) -> Key {
        Key {
            source: source.into(),
            palette: Palette::for_theme(crate::theme::Theme::default()),
            graphical,
        }
    }

    fn gallery() -> Vec<String> {
        let mut blocks = Vec::new();
        let mut current = None;
        for event in pulldown_cmark::Parser::new(include_str!("../examples/mermaid.md")) {
            match event {
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(_)) => {
                    current = Some(String::new())
                }
                pulldown_cmark::Event::Text(s) if current.is_some() => {
                    current.as_mut().unwrap().push_str(&s)
                }
                pulldown_cmark::Event::End(pulldown_cmark::TagEnd::CodeBlock) => {
                    blocks.push(current.take().unwrap())
                }
                _ => {}
            }
        }
        blocks
    }

    #[test]
    fn graphical_gallery_is_valid_png_in_light_and_dark_themes() {
        let output = std::env::var_os("VAAYU_MERMAID_GALLERY").map(std::path::PathBuf::from);
        if let Some(dir) = &output {
            std::fs::create_dir_all(dir).unwrap();
        }
        let sources = gallery();
        assert_eq!(sources.len(), 6);
        for light in [false, true] {
            for (i, source) in sources.iter().enumerate() {
                let mut k = key(source, true);
                if light {
                    k.palette = Palette {
                        light: true,
                        bg: [248, 250, 252],
                        fg: [15, 23, 42],
                        accent: [3, 105, 161],
                    };
                }
                let start = Instant::now();
                let artifact = render_result(
                    &k,
                    OperationControl::new().with_deadline(Duration::from_secs(3)),
                )
                .unwrap_or_else(|e| panic!("gallery {i}, light={light}: {e}"));
                let Artifact::Image {
                    png, width, height, ..
                } = artifact
                else {
                    panic!("expected real image");
                };
                assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
                assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), width);
                assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), height);
                assert!(width > 100 && height > 100 && png.len() > 2000);
                let mut decoder = png::Decoder::new(std::io::Cursor::new(&png))
                    .read_info()
                    .unwrap();
                let mut pixels = vec![0; decoder.output_buffer_size().unwrap()];
                decoder.next_frame(&mut pixels).unwrap();
                assert_eq!(
                    &pixels[..3],
                    &k.palette.bg,
                    "diagram canvas must match its palette"
                );
                assert!(
                    width <= 2400 && height <= 3300,
                    "raster exceeded fit budget"
                );
                if let Some(dir) = &output {
                    let mode = if light { "light" } else { "dark" };
                    std::fs::write(dir.join(format!("{i}-{mode}.png")), &png).unwrap();
                }
                eprintln!(
                    "gallery {i}, light={light}: {width}×{height}, {:?}",
                    start.elapsed()
                );
            }
        }
    }

    #[test]
    fn unicode_gallery_preserves_labels_and_runtime_flow() {
        let labels = [
            "Client",
            "Fetch profile",
            "Published",
            "Document",
            "DOCUMENT",
        ];
        for (source, label) in gallery().iter().zip(labels) {
            let artifact = render_result(&key(source, false), OperationControl::new()).unwrap();
            let Artifact::Text(rows) = artifact else {
                panic!("expected terminal diagram");
            };
            assert!(rows.join("\n").contains(label));
            assert!(!rows.join("\n").contains('\x1b'));
        }
        let source = include_str!("../TECHNICAL_DECISIONS.md")
            .split("```mermaid\n")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap();
        let Artifact::Text(rows) =
            render_result(&key(source, false), OperationControl::new()).unwrap()
        else {
            panic!("expected text");
        };
        let text = rows.join("\n");
        for label in ["Still current?", "Discard stale", "Write only"] {
            assert!(text.contains(label), "{label} was lost: {text}");
        }
    }

    #[test]
    fn invalid_oversized_and_cancelled_diagrams_report_errors() {
        for source in [
            "flowchart TD\nA[",
            "not-a-diagram",
            "flowchart TD\nA[\x1b] --> B",
        ] {
            let artifact = render(&key(source, false), OperationControl::new());
            if let Artifact::Error(error) = artifact {
                assert!(!error.contains('\x1b'));
            } else if source != "flowchart TD\nA[\x1b] --> B" {
                panic!("invalid diagram accepted");
            }
        }
        let k = key(
            &format!("flowchart TD\n{}", "A --> B\n".repeat(10_000)),
            false,
        );
        assert!(matches!(
            render(&k, OperationControl::new()),
            Artifact::Error(_)
        ));
        let control = OperationControl::new();
        control.cancel();
        assert!(matches!(
            render(&key("flowchart TD\nA --> B", true), control),
            Artifact::Error(_)
        ));
    }

    #[test]
    fn reopening_the_same_diagram_retries_after_a_cancelled_worker_reply() {
        let mut service = Service::default();
        let key = key("flowchart LR\nA --> B", false);
        let (tx, jobs) = mpsc::sync_channel(1);
        let (replies, rx) = mpsc::channel();
        service.worker = Some(Worker { tx, rx });
        service.begin_frame();
        service.request(&key);
        service.active = Some((key.clone(), OperationControl::new()));
        service.begin_frame();
        service.end_frame();
        assert!(service.active.as_ref().unwrap().1.is_cancelled());
        service.begin_frame();
        service.request(&key);
        replies
            .send((
                key.clone(),
                Arc::new(Artifact::Error("obsolete cancellation".into())),
            ))
            .unwrap();
        service.poll();
        assert!(
            service.request(&key).is_none(),
            "old cancellation replaced reopened request"
        );
        service.cache.get_mut(&key).unwrap().queued = Instant::now() - DEBOUNCE;
        service.poll();
        let (retried, control) = jobs
            .try_recv()
            .expect("reopened diagram must be rescheduled");
        assert_eq!(retried.source, key.source);
        assert!(!control.is_cancelled());
    }

    #[test]
    fn stale_worker_result_cannot_replace_new_source_and_artifacts_are_shared() {
        let mut service = Service::default();
        let old = key("flowchart LR\nA[Old label] --> B", false);
        let new = key("flowchart LR\nA[Latest label] --> B", false);
        service.begin_frame();
        service.request(&old);
        service.cache.get_mut(&old).unwrap().queued = Instant::now() - DEBOUNCE;
        service.poll();
        service.begin_frame();
        service.request(&new);
        let revision = service.revision;
        service.end_frame();
        assert!(
            service.revision > revision,
            "cancelled requests invalidate saved layouts"
        );
        service.cache.get_mut(&new).unwrap().queued = Instant::now() - DEBOUNCE;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            service.poll();
            if service.cache.get(&new).unwrap().result.is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "worker did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!service.cache.contains_key(&old));
        let a = service.request(&new).unwrap();
        let b = service.request(&new).unwrap();
        assert!(
            Arc::ptr_eq(&a, &b),
            "same diagram should reuse its artifact"
        );
        let Artifact::Text(rows) = a.as_ref() else {
            panic!("expected diagram");
        };
        assert!(rows.join("\n").contains("Latest label"));
        assert!(!rows.join("\n").contains("Old label"));
    }
}
