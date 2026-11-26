//! GPU textures of finished thumbnails, shared by all windows, least recently used ones evicted.

use eframe::egui::{self, ColorImage, Context, TextureHandle, TextureId, TextureOptions, Vec2};
use fx_app::{Explorer, Thumb, ThumbKey};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// About 1000 thumbnails at 256 px; enough for several screens of scrolling back and forth.
const BUDGET_BYTES: usize = 256 * 1024 * 1024;
/// Uploading is a copy to the GPU per texture; spreading a burst over frames keeps scrolling smooth.
const UPLOADS_PER_FRAME: usize = 12;

struct Slot {
    texture: TextureHandle,
    /// Size in physical pixels.
    size: Vec2,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
struct Cache {
    slots: HashMap<ThumbKey, Slot>,
    waiting: VecDeque<(ThumbKey, Arc<Thumb>)>,
    bytes: usize,
    frame: u64,
}

type Shared = Arc<Mutex<Cache>>;

fn shared(ctx: &Context) -> Shared {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(egui::Id::unique("fx-thumbnails"), Shared::default)
            .clone()
    })
}

/// Takes the explorer's finished thumbnails and uploads a few of them; call once per frame before [`get`].
pub fn begin_frame(ctx: &Context, explorer: &mut Explorer) {
    let shared = shared(ctx);
    let mut cache = shared.lock().unwrap_or_else(|e| e.into_inner());
    cache.frame = ctx.cumulative_frame_nr();
    cache.waiting.extend(explorer.take_thumbnails());
    for _ in 0..UPLOADS_PER_FRAME {
        let Some((key, thumb)) = cache.waiting.pop_front() else {
            break;
        };
        upload(ctx, &mut cache, key, &thumb);
    }
    if !cache.waiting.is_empty() {
        ctx.request_repaint();
    }
    evict(&mut cache);
}

fn upload(ctx: &Context, cache: &mut Cache, key: ThumbKey, thumb: &Thumb) {
    let (w, h) = (thumb.width as usize, thumb.height as usize);
    if w * h * 4 != thumb.rgba.len() {
        return;
    }
    let image = ColorImage::from_rgba_unmultiplied([w, h], &thumb.rgba);
    let name = format!("thumb:{}", key.path.display());
    let texture = ctx.load_texture(name, image, TextureOptions::LINEAR);
    let bytes = thumb.rgba.len();
    cache.bytes += bytes;
    let slot = Slot {
        texture,
        size: Vec2::new(w as f32, h as f32),
        bytes,
        used: cache.frame,
    };
    if let Some(old) = cache.slots.insert(key, slot) {
        cache.bytes -= old.bytes;
    }
}

/// Drops the least recently used textures not shown this frame until the cache fits its budget.
fn evict(cache: &mut Cache) {
    if cache.bytes <= BUDGET_BYTES {
        return;
    }
    let mut stale: Vec<(u64, ThumbKey)> = cache
        .slots
        .iter()
        .filter(|(_, s)| s.used < cache.frame)
        .map(|(k, s)| (s.used, k.clone()))
        .collect();
    stale.sort_unstable_by_key(|(used, _)| *used);
    for (_, key) in stale {
        if cache.bytes <= BUDGET_BYTES {
            break;
        }
        if let Some(slot) = cache.slots.remove(&key) {
            cache.bytes -= slot.bytes;
        }
    }
}

/// This frame's view of the cache, fetched once so tiles do not go through the context each.
pub struct Textures(Shared);

pub fn textures(ctx: &Context) -> Textures {
    Textures(shared(ctx))
}

impl Textures {
    /// The texture of `key` and its size in physical pixels, if uploaded.
    pub fn get(&self, key: &ThumbKey) -> Option<(TextureId, Vec2)> {
        let mut cache = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let frame = cache.frame;
        let slot = cache.slots.get_mut(key)?;
        slot.used = frame;
        Some((slot.texture.id(), slot.size))
    }

    /// True if `key` is uploaded or waiting for its upload, so it need not be produced again.
    pub fn known(&self, key: &ThumbKey) -> bool {
        let cache = self.0.lock().unwrap_or_else(|e| e.into_inner());
        cache.slots.contains_key(key) || cache.waiting.iter().any(|(k, _)| k == key)
    }
}
