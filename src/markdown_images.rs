//! Bounded, asynchronous image loading for Markdown previews. The UI owns
//! demand/cache bookkeeping; one lazy worker owns file/network I/O and decoding.
use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    time::{Duration, Instant, SystemTime},
};

use crate::graphics::Image;
use image::ImageDecoder;

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_CACHE: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 64;
const DEBOUNCE: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    File(PathBuf),
    Http(String),
}

impl Source {
    pub fn resolve(destination: &str, document: Option<&Path>) -> Result<Self, String> {
        let directory = document
            .map(crate::files::identity)
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let base = url::Url::from_directory_path(directory)
            .map_err(|_| "cannot resolve image directory")?;
        let mut url = base.join(destination).map_err(|e| e.to_string())?;
        url.set_fragment(None);
        match url.scheme() {
            "file" => url
                .to_file_path()
                .map(Self::File)
                .map_err(|_| "invalid local image path".into()),
            "http" | "https" => Ok(Self::Http(url.into())),
            scheme => Err(format!("unsupported image URL scheme: {scheme}")),
        }
    }

    fn stamp(&self) -> Option<(SystemTime, u64)> {
        match self {
            Self::File(path) => {
                let metadata = std::fs::metadata(path).ok()?;
                Some((metadata.modified().ok()?, metadata.len()))
            }
            Self::Http(_) => None,
        }
    }
}

pub type Artifact = Result<Arc<Image>, String>;

struct Entry {
    queued: Instant,
    used: u64,
    generation: u64,
    stamp: Option<(SystemTime, u64)>,
    result: Option<Artifact>,
}

type Job = (Source, u64);
struct Worker {
    tx: mpsc::SyncSender<Job>,
    rx: mpsc::Receiver<(Job, Artifact)>,
}

#[derive(Default)]
pub struct Service {
    cache: HashMap<Source, Entry>,
    demand: HashSet<Source>,
    worker: Option<Worker>,
    active: Option<Job>,
    pub revision: u64,
    frame: u64,
    generation: u64,
    last_check: Option<Instant>,
}

impl Service {
    pub fn has_demand(&self) -> bool {
        !self.demand.is_empty()
    }

    pub fn begin_frame(&mut self) {
        self.demand.clear();
        self.frame += 1;
    }

    pub fn touch(&mut self, source: &Source) {
        self.demand.insert(source.clone());
        if let Some(entry) = self.cache.get_mut(source) {
            entry.used = self.frame;
        }
    }

    pub fn request(&mut self, source: &Source) -> Option<Artifact> {
        self.touch(source);
        if !self.cache.contains_key(source) {
            while self.cache.len() >= MAX_ENTRIES {
                if !self.evict_one() {
                    return Some(Err("too many images in preview (limit 64)".into()));
                }
            }
            self.generation += 1;
            self.cache.insert(
                source.clone(),
                Entry {
                    queued: Instant::now(),
                    used: self.frame,
                    generation: self.generation,
                    stamp: source.stamp(),
                    result: None,
                },
            );
        }
        self.cache.get(source).and_then(|e| e.result.clone())
    }

    pub fn end_frame(&mut self) {
        let previous = self.cache.len();
        self.cache
            .retain(|source, entry| entry.result.is_some() || self.demand.contains(source));
        if previous != self.cache.len() {
            self.revision += 1;
        }
    }

    fn evict_one(&mut self) -> bool {
        let victim = self
            .cache
            .iter()
            .filter(|(source, _)| !self.demand.contains(*source))
            .min_by_key(|(_, entry)| entry.used)
            .map(|(source, _)| source.clone());
        if let Some(source) = victim {
            self.cache.remove(&source);
            self.revision += 1;
            true
        } else {
            false
        }
    }

    pub fn image(&self, id: u32) -> Option<Arc<Image>> {
        self.cache
            .values()
            .filter_map(|entry| entry.result.as_ref())
            .find_map(|result| result.as_ref().ok().filter(|image| image.id == id).cloned())
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let replies: Vec<_> = self
            .worker
            .as_ref()
            .map(|w| w.rx.try_iter().collect())
            .unwrap_or_default();
        for ((source, generation), mut artifact) in replies {
            self.active = None;
            if !self
                .cache
                .get(&source)
                .is_some_and(|e| e.generation == generation)
            {
                continue;
            }
            let bytes = artifact.as_ref().map_or(0, |image| image.png.len());
            while self
                .cache
                .values()
                .filter_map(|e| e.result.as_ref())
                .map(|r| r.as_ref().map_or(0, |image| image.png.len()))
                .sum::<usize>()
                + bytes
                > MAX_CACHE
            {
                if !self.evict_one() {
                    artifact = Err("preview image cache exceeds 32 MiB budget".into());
                    break;
                }
            }
            if let Some(entry) = self.cache.get_mut(&source) {
                entry.result = Some(artifact);
                self.revision += 1;
                changed = true;
            }
        }
        // Also refresh edited/replaced/missing local images while the preview
        // is idle. Never stat the whole cache on each 10 ms event-loop tick.
        if self
            .last_check
            .is_none_or(|t| t.elapsed() >= Duration::from_millis(500))
        {
            self.last_check = Some(Instant::now());
            for (source, entry) in &mut self.cache {
                if !self.demand.contains(source) {
                    continue;
                }
                let stamp = source.stamp();
                if entry.stamp != stamp {
                    self.generation += 1;
                    entry.generation = self.generation;
                    entry.stamp = stamp;
                    entry.result = None;
                    entry.queued = Instant::now();
                    self.revision += 1;
                    changed = true;
                }
            }
        }
        if self.active.is_some() {
            return changed;
        }
        let next = self
            .cache
            .iter()
            .filter(|(source, entry)| {
                entry.result.is_none()
                    && self.demand.contains(*source)
                    && entry.queued.elapsed() >= DEBOUNCE
            })
            .min_by_key(|(_, entry)| entry.queued)
            .map(|(source, entry)| (source.clone(), entry.generation));
        if let Some(job) = next {
            let worker = self.worker.get_or_insert_with(|| {
                let (tx, jobs) = mpsc::sync_channel::<Job>(1);
                let (results, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let agent: ureq::Agent = ureq::Agent::config_builder()
                        .timeout_global(Some(Duration::from_secs(5)))
                        .build()
                        .into();
                    while let Ok(job) = jobs.recv() {
                        let artifact = load(&job.0, &agent).map_err(|error| {
                            crate::mermaid::clean_text(&error)
                                .chars()
                                .take(240)
                                .collect()
                        });
                        if results.send((job, artifact)).is_err() {
                            break;
                        }
                    }
                });
                Worker { tx, rx }
            });
            if worker.tx.try_send(job.clone()).is_ok() {
                self.active = Some(job);
            }
        }
        changed
    }
}

fn load(source: &Source, agent: &ureq::Agent) -> Artifact {
    let bytes = match source {
        Source::File(path) => {
            let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
            if !metadata.is_file() {
                return Err("image path is not a regular file".into());
            }
            if metadata.len() > MAX_BYTES as u64 {
                return Err("image exceeds 16 MiB limit".into());
            }
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            file.take(MAX_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            bytes
        }
        Source::Http(url) => agent
            .get(url)
            .call()
            .map_err(|e| e.to_string())?
            .body_mut()
            .with_config()
            .limit(MAX_BYTES as u64 + 1)
            .read_to_vec()
            .map_err(|e| e.to_string())?,
    };
    if bytes.len() > MAX_BYTES {
        return Err("image exceeds 16 MiB limit".into());
    }
    decode(&bytes).map(Arc::new)
}

fn decode(bytes: &[u8]) -> Result<Image, String> {
    // SVG is rasterized locally; embedded resource URLs are deliberately not
    // resolved. Raster formats are detected by content, not the extension.
    if std::str::from_utf8(bytes).is_ok_and(|s| {
        s.trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with('<')
    }) {
        let mut options = resvg::usvg::Options {
            image_href_resolver: resvg::usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            },
            ..Default::default()
        };
        options.fontdb_mut().load_system_fonts();
        let tree = resvg::usvg::Tree::from_data(bytes, &options).map_err(|e| e.to_string())?;
        let size = tree.size();
        let scale = (2400.0 / size.width()).min(3300.0 / size.height()).min(1.0);
        let width = (size.width() * scale).ceil().max(1.0) as u32;
        let height = (size.height() * scale).ceil().max(1.0) as u32;
        let mut pixmap =
            resvg::tiny_skia::Pixmap::new(width, height).ok_or("image dimensions too large")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        let png = pixmap.encode_png().map_err(|e| e.to_string())?;
        if png.len() > MAX_BYTES {
            return Err("rendered image exceeds 16 MiB limit".into());
        }
        return Image::new(png, width, height);
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    // Match ImageReader::decode's output-buffer reservation, which is not
    // performed by into_decoder (used here to retrieve EXIF orientation).
    limits
        .reserve(decoder.total_bytes())
        .map_err(|e| e.to_string())?;
    decoder.set_limits(limits).map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut decoded = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    decoded.apply_orientation(orientation);
    if decoded.width() > 2400 || decoded.height() > 3300 {
        decoded = decoded.thumbnail(2400, 3300);
    }
    let decoded = decoded.to_rgba8();
    let (width, height) = decoded.dimensions();
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(decoded)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    if png.get_ref().len() > MAX_BYTES {
        return Err("rendered image exceeds 16 MiB limit".into());
    }
    Image::new(png.into_inner(), width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(format: image::ImageFormat) -> Vec<u8> {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            32,
            16,
            image::Rgb([220, 20, 30]),
        ));
        let image = if format == image::ImageFormat::Ico {
            image::DynamicImage::ImageRgba8(image.to_rgba8())
        } else {
            image
        };
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn sources_resolve_relative_to_the_document_and_decode_url_paths() {
        let document = std::env::temp_dir().join("vaayu-image-paths/docs/readme.md");
        let root = document.parent().unwrap().parent().unwrap();
        assert_eq!(
            Source::resolve("../assets/my%20photo.png#preview", Some(&document)).unwrap(),
            Source::File(root.join("assets/my photo.png"))
        );
        let path = root.join("absolute.png");
        assert_eq!(
            Source::resolve(
                url::Url::from_file_path(&path).unwrap().as_str(),
                Some(&document)
            )
            .unwrap(),
            Source::File(path)
        );
        assert_eq!(
            Source::resolve("https://example.com/image?size=2#fragment", Some(&document)).unwrap(),
            Source::Http("https://example.com/image?size=2".into())
        );
        assert!(Source::resolve("javascript:alert(1)", Some(&document)).is_err());
    }

    #[test]
    fn raster_formats_are_transcoded_without_upscaling() {
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::Gif,
            image::ImageFormat::WebP,
            image::ImageFormat::Bmp,
            image::ImageFormat::Ico,
        ] {
            let output = decode(&encoded(format)).unwrap_or_else(|e| panic!("{format:?}: {e}"));
            assert_eq!((output.width, output.height), (32, 16));
            let pixels = image::load_from_memory(&output.png).unwrap().to_rgba8();
            assert!(
                pixels.get_pixel(0, 0)[0] > 200,
                "{format:?} lost image pixels"
            );
            assert_eq!(pixels.get_pixel(0, 0)[3], 255);
        }
    }

    #[test]
    fn oversized_raster_output_is_rejected_before_allocating_pixels() {
        let mut bmp = encoded(image::ImageFormat::Bmp);
        bmp[18..22].copy_from_slice(&10_000_i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&10_000_i32.to_le_bytes());
        let error = match decode(&bmp) {
            Ok(_) => panic!("oversized BMP should exceed the output-buffer budget"),
            Err(error) => error,
        };
        assert!(error.contains("Memory limit"), "{error}");
    }

    #[test]
    fn transparency_and_svg_geometry_are_preserved() {
        let input = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            4,
            image::Rgba([80, 90, 100, 64]),
        ));
        let mut bytes = Cursor::new(Vec::new());
        input.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let output = decode(bytes.get_ref()).unwrap();
        let pixels = image::load_from_memory(&output.png).unwrap().to_rgba8();
        assert_eq!(pixels.get_pixel(0, 0).0, [80, 90, 100, 64]);
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="40" height="40" fill="#ff0000"/></svg>"##;
        let output = decode(svg).unwrap();
        assert_eq!((output.width, output.height), (80, 40));
        let pixels = image::load_from_memory(&output.png).unwrap().to_rgba8();
        assert_eq!(pixels.get_pixel(10, 10).0, [255, 0, 0, 255]);
        assert_eq!(pixels.get_pixel(70, 10)[3], 0);
        assert!(decode(b"not an image").is_err());
        assert!(decode(&encoded(image::ImageFormat::Png)[..24]).is_err());
    }

    #[test]
    fn cancelled_and_reopened_requests_reject_old_worker_results() {
        let source = Source::Http("https://example.com/image.png".into());
        let mut service = Service::default();
        service.begin_frame();
        assert!(service.request(&source).is_none());
        let old_generation = service.cache[&source].generation;
        service.begin_frame();
        service.end_frame();
        assert!(!service.cache.contains_key(&source));
        service.begin_frame();
        service.request(&source);
        let new_generation = service.cache[&source].generation;
        assert_ne!(new_generation, old_generation);
        let (tx, _) = mpsc::sync_channel(1);
        let (results, rx) = mpsc::channel();
        service.worker = Some(Worker { tx, rx });
        results
            .send(((source.clone(), old_generation), Err("stale".into())))
            .unwrap();
        service.poll();
        assert!(service.cache[&source].result.is_none());
        let image = Arc::new(decode(&encoded(image::ImageFormat::Png)).unwrap());
        results
            .send(((source.clone(), new_generation), Ok(image.clone())))
            .unwrap();
        assert!(service.poll());
        assert_eq!(service.request(&source).unwrap().unwrap().id, image.id);
        assert_eq!(service.image(image.id).unwrap().width, 32);
    }

    #[test]
    fn cache_entries_are_bounded_and_unused_images_can_be_evicted() {
        let mut service = Service::default();
        service.begin_frame();
        for i in 0..MAX_ENTRIES {
            assert!(service
                .request(&Source::Http(format!("https://example.com/{i}")))
                .is_none());
        }
        let next = Source::Http("https://example.com/extra".into());
        assert!(service.request(&next).unwrap().is_err());
        service.begin_frame();
        assert!(service.request(&next).is_none());
        assert_eq!(service.cache.len(), MAX_ENTRIES);
    }

    #[test]
    fn local_changes_and_missing_files_invalidate_cached_results() {
        let image = Arc::new(decode(&encoded(image::ImageFormat::Png)).unwrap());
        let path = std::env::temp_dir().join(format!(
            "vaayu-image-{}-{}.png",
            std::process::id(),
            image.id
        ));
        let source = Source::File(path.clone());
        let mut service = Service::default();
        service.begin_frame();
        service.request(&source);
        service.cache.get_mut(&source).unwrap().result = Some(Err("missing".into()));
        std::fs::write(&path, &image.png).unwrap();
        assert!(service.poll());
        assert!(service.cache[&source].result.is_none());
        service.cache.get_mut(&source).unwrap().result = Some(Ok(image));
        std::fs::remove_file(path).unwrap();
        service.last_check = None;
        assert!(service.poll());
        assert!(service.cache[&source].result.is_none());
    }
}
