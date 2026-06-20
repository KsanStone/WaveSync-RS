use std::sync::Arc;

#[derive(Clone)]
pub struct MediaArtwork {
    pub revision: u64,
    pub rgba: Arc<[u8]>,
    pub width: usize,
    pub height: usize,
}

#[derive(Clone)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub position: std::time::Duration,
    pub duration: std::time::Duration,
    pub artwork: Option<MediaArtwork>,
}

/// Platform-neutral source for the media card.
///
/// Implementations must return quickly; platform APIs and image decoding belong
/// on a worker thread rather than the UI thread.
pub trait MediaProvider: Send + Sync {
    fn current_media(&self) -> Option<MediaInfo>;
}

pub fn platform_media_provider() -> Box<dyn MediaProvider> {
    Box::new(PlatformMediaProvider::new())
}

#[cfg(not(target_os = "windows"))]
struct PlatformMediaProvider;

#[cfg(not(target_os = "windows"))]
impl PlatformMediaProvider {
    fn new() -> Self {
        Self
    }
}

#[cfg(not(target_os = "windows"))]
impl MediaProvider for PlatformMediaProvider {
    fn current_media(&self) -> Option<MediaInfo> {
        None
    }
}

#[cfg(target_os = "windows")]
mod windows_provider {
    use super::{MediaArtwork, MediaInfo, MediaProvider};
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::sync::{Arc, RwLock, Weak};
    use std::time::Duration;
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;
    use windows::Storage::Streams::{DataReader, IRandomAccessStreamWithContentType};
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

    pub(super) struct PlatformMediaProvider {
        cached: Arc<RwLock<Option<MediaInfo>>>,
    }

    impl PlatformMediaProvider {
        pub(super) fn new() -> Self {
            let cached = Arc::new(RwLock::new(None));
            let worker_cache = Arc::downgrade(&cached);
            std::thread::Builder::new()
                .name("media-session".into())
                .spawn(move || poll_media(worker_cache))
                .expect("failed to start media-session worker");
            Self { cached }
        }
    }

    impl MediaProvider for PlatformMediaProvider {
        fn current_media(&self) -> Option<MediaInfo> {
            self.cached.read().ok()?.clone()
        }
    }

    fn poll_media(cache: Weak<RwLock<Option<MediaInfo>>>) {
        struct ComApartment;
        impl Drop for ComApartment {
            fn drop(&mut self) {
                unsafe { CoUninitialize() };
            }
        }
        let _apartment = match unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
            Ok(()) => ComApartment,
            Err(error) => {
                log::warn!("Could not initialize WinRT media worker: {error}");
                return;
            }
        };

        let manager = loop {
            let Some(cache) = cache.upgrade() else { return };
            match GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
                .and_then(|operation| operation.get())
            {
                Ok(manager) => break manager,
                Err(error) => {
                    *cache.write().unwrap() = None;
                    log::debug!("WinRT media session manager unavailable: {error}");
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        };

        let mut artwork_key = None;
        let mut artwork = None;
        loop {
            let Some(cache) = cache.upgrade() else { return };
            let next = read_current_media(&manager, &mut artwork_key, &mut artwork)
                .inspect_err(|error| log::debug!("Could not read WinRT media session: {error}"))
                .ok()
                .flatten();
            *cache.write().unwrap() = next;
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    fn read_current_media(
        manager: &GlobalSystemMediaTransportControlsSessionManager,
        artwork_key: &mut Option<u64>,
        artwork: &mut Option<MediaArtwork>,
    ) -> windows::core::Result<Option<MediaInfo>> {
        let session = match manager.GetCurrentSession() {
            Ok(session) => session,
            Err(_) => return Ok(None),
        };
        let properties = session.TryGetMediaPropertiesAsync()?.get()?;
        let title = properties.Title()?.to_string();
        if title.trim().is_empty() {
            return Ok(None);
        }
        let artist = properties.Artist()?.to_string();
        let timeline = session.GetTimelineProperties()?;
        let position = win_duration(timeline.Position()?.Duration);
        let duration = win_duration(timeline.EndTime()?.Duration.max(0));

        let mut hasher = DefaultHasher::new();
        title.hash(&mut hasher);
        artist.hash(&mut hasher);
        properties.AlbumTitle()?.to_string().hash(&mut hasher);
        let key = hasher.finish();
        if *artwork_key != Some(key) {
            *artwork_key = Some(key);
            *artwork = read_artwork(&properties, key).ok().flatten();
        }

        Ok(Some(MediaInfo {
            title,
            artist,
            position,
            duration,
            artwork: artwork.clone(),
        }))
    }

    fn win_duration(ticks: i64) -> Duration {
        Duration::from_nanos(ticks.max(0) as u64 * 100)
    }

    fn read_artwork(
        properties: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
        revision: u64,
    ) -> windows::core::Result<Option<MediaArtwork>> {
        let reference = match properties.Thumbnail() {
            Ok(reference) => reference,
            Err(_) => return Ok(None),
        };
        let stream: IRandomAccessStreamWithContentType = reference.OpenReadAsync()?.get()?;
        let size = stream.Size()?;
        if size == 0 || size > 20 * 1024 * 1024 {
            return Ok(None);
        }
        let reader = DataReader::CreateDataReader(&stream)?;
        reader.LoadAsync(size as u32)?.get()?;
        let mut encoded = vec![0; size as usize];
        reader.ReadBytes(&mut encoded)?;
        let decoded = match image::load_from_memory(&encoded) {
            Ok(image) => image.into_rgba8(),
            Err(_) => return Ok(None),
        };
        let (width, height) = decoded.dimensions();
        Ok(Some(MediaArtwork {
            revision,
            rgba: Arc::from(decoded.into_raw()),
            width: width as usize,
            height: height as usize,
        }))
    }
}

#[cfg(target_os = "windows")]
use windows_provider::PlatformMediaProvider;
