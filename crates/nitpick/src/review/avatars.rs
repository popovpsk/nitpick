use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use avatar_cache::{Auth, Cache, Downloader};
use forge::ForgeKind;
use gpui::{
    AnyElement, App, AppContext as _, FontWeight, Global, Image, ImageFormat, IntoElement,
    ParentElement as _, Pixels, SharedString, Styled as _, div, img,
};
use gpui_component::ActiveTheme as _;

use super::blame::tint;
use crate::settings_store::{app_dir, settings};

enum Slot {
    Loading,
    Ready(Arc<Image>),
    Missing,
}

struct Avatars {
    slots: HashMap<SharedString, Slot>,
    cache: Arc<Cache>,
    downloader: Option<Arc<Downloader>>,
    refresh_pending: bool,
}

const REFRESH_DELAY: Duration = Duration::from_millis(100);

impl Global for Avatars {}

pub fn init(cx: &mut App) {
    let downloader = Downloader::new()
        .inspect_err(|err| log::warn!("avatars: {err}"))
        .ok()
        .map(Arc::new);
    cx.set_global(Avatars {
        slots: HashMap::new(),
        cache: Arc::new(Cache::new(app_dir(dirs::data_dir()).join("avatars"))),
        downloader,
        refresh_pending: false,
    });
}

pub fn request(urls: impl IntoIterator<Item = SharedString>, cx: &mut App) {
    let Some(avatars) = cx.try_global::<Avatars>() else {
        return;
    };
    let cache = avatars.cache.clone();
    let downloader = avatars.downloader.clone();
    for url in urls {
        if matches!(
            cx.global::<Avatars>().slots.get(&url),
            Some(Slot::Loading | Slot::Ready(_))
        ) {
            continue;
        }
        cx.global_mut::<Avatars>()
            .slots
            .insert(url.clone(), Slot::Loading);
        let auth = auth_for(&url, cx);
        let cache = cache.clone();
        let downloader = downloader.clone();
        cx.spawn(async move |cx| {
            let png = load(url.to_string(), auth, cache, downloader, cx).await;
            cx.update(|cx| {
                let slot = match png {
                    Some(png) => Slot::Ready(Arc::new(Image::from_bytes(ImageFormat::Png, png))),
                    None => Slot::Missing,
                };
                let avatars = cx.global_mut::<Avatars>();
                avatars.slots.insert(url, slot);
                if !avatars.refresh_pending {
                    avatars.refresh_pending = true;
                    cx.spawn(async move |cx| {
                        cx.background_executor().timer(REFRESH_DELAY).await;
                        cx.update(|cx| {
                            cx.global_mut::<Avatars>().refresh_pending = false;
                            cx.refresh_windows();
                        });
                    })
                    .detach();
                }
            });
        })
        .detach();
    }
}

async fn load(
    url: String,
    auth: Option<Auth>,
    cache: Arc<Cache>,
    downloader: Option<Arc<Downloader>>,
    cx: &mut gpui::AsyncApp,
) -> Option<Vec<u8>> {
    let cached = cx
        .background_spawn({
            let cache = cache.clone();
            let url = url.clone();
            async move { cache.read(&url, SystemTime::now()) }
        })
        .await;
    let stale = match cached {
        Some(cached) if cached.fresh => return Some(cached.png),
        Some(cached) => Some(cached.png),
        None => None,
    };
    let Some(downloader) = downloader else {
        return stale;
    };
    let fetched = net_runtime::spawn({
        let url = url.clone();
        async move { downloader.fetch(&url, auth.as_ref()).await }
    })
    .await;
    let raw = match fetched {
        Ok(raw) => raw,
        Err(err) => {
            log::debug!("avatar {url}: {err}");
            return stale;
        }
    };
    cx.background_spawn(async move {
        match avatar_cache::normalize(&raw) {
            Ok(png) => {
                if let Err(err) = cache.write(&url, &png) {
                    log::debug!("avatar cache {url}: {err}");
                }
                Some(png)
            }
            Err(err) => {
                log::debug!("avatar {url}: {err}");
                stale
            }
        }
    })
    .await
}

fn auth_for(url: &str, cx: &App) -> Option<Auth> {
    let current = settings(cx);
    [ForgeKind::GitLab, ForgeKind::GitHub]
        .into_iter()
        .filter(|kind| current.forge_configured(*kind))
        .filter_map(|kind| {
            let base = current.forge_url(kind);
            let token = current.forge_token(kind);
            match kind {
                ForgeKind::GitLab => Auth::gitlab(&base, token),
                ForgeKind::GitHub => Auth::github(&base, token),
            }
        })
        .find(|auth| auth.applies_to(url))
}

pub fn render(
    url: Option<&SharedString>,
    initials: &SharedString,
    tint_ix: usize,
    size: Pixels,
    cx: &App,
) -> AnyElement {
    let image = url
        .and_then(|url| cx.try_global::<Avatars>()?.slots.get(url))
        .and_then(|slot| match slot {
            Slot::Ready(image) => Some(image.clone()),
            Slot::Loading | Slot::Missing => None,
        });
    if let Some(image) = image {
        return img(image)
            .size(size)
            .flex_none()
            .rounded_full()
            .into_any_element();
    }
    let theme = cx.theme();
    div()
        .size(size)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .rounded_full()
        .bg(tint(theme, tint_ix))
        .text_color(theme.foreground)
        .text_size(size * 0.42)
        .font_weight(FontWeight::SEMIBOLD)
        .child(initials.clone())
        .into_any_element()
}

#[derive(Clone)]
pub struct Face {
    pub avatar: Option<SharedString>,
    pub initials: SharedString,
    pub tint: usize,
}

impl Face {
    pub fn of(user: &forge::User) -> Self {
        Self {
            avatar: user.avatar_url.clone().map(SharedString::from),
            initials: review_core::blame::initials(&user.name).into(),
            tint: review_core::blame::tint_index(&user.username, super::blame::TINT_COUNT),
        }
    }

    pub fn render(&self, size: Pixels, cx: &App) -> AnyElement {
        render(self.avatar.as_ref(), &self.initials, self.tint, size, cx)
    }
}
