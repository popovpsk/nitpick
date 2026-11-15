use std::time::{Duration, Instant};

use gpui::{AnyElement, Context, IntoElement, ParentElement as _, Styled as _, Window, div, px};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, button::Button, h_flex};
use ui_resources::Icons;

use super::{Refresh, ReviewSource, ReviewView};
use crate::localization::tr;

const POLL_EVERY: Duration = Duration::from_secs(300);
const TICK: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct PushWatch {
    pub last_check: Option<Instant>,
    pub checking: bool,
    pub pushed: bool,
}

impl ReviewView {
    pub(super) fn start_push_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.source, ReviewSource::MergeRequest { .. }) {
            return;
        }
        self._push_watch = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = this.update_in(cx, |this, window, cx| {
                    if window.is_window_active() {
                        this.check_for_push(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
    }

    pub(super) fn check_for_push(&mut self, cx: &mut Context<Self>) {
        let Some(ctx) = self.mr.clone() else {
            return;
        };
        let watch = &self.push_watch;
        if !self.request_open()
            || watch.pushed
            || watch.checking
            || self.busy.is_some()
            || watch.last_check.is_some_and(|at| at.elapsed() < POLL_EVERY)
        {
            return;
        }
        self.push_watch.last_check = Some(Instant::now());
        self.push_watch.checking = true;
        let loaded = ctx.refs.head_sha.clone();
        self._push_check = cx.spawn(async move |this, cx| {
            let result = net_runtime::spawn(async move {
                ctx.client.request(&ctx.project, ctx.request.number).await
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.push_watch.checking = false;
                match result {
                    Ok(request) => {
                        let head = request.diff_refs.map(|refs| refs.head_sha);
                        let still_loaded = this
                            .mr
                            .as_ref()
                            .is_some_and(|ctx| ctx.refs.head_sha == loaded);
                        if still_loaded && head.is_some_and(|head| head != loaded) {
                            this.push_watch.pushed = true;
                            cx.notify();
                        }
                    }
                    Err(err) => log::debug!("checking for new commits: {err}"),
                }
            });
        });
    }

    pub(super) fn note_request_loaded(&mut self) {
        self.push_watch.last_check = Some(Instant::now());
        self.push_watch.pushed = false;
    }

    pub(super) fn render_push_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.push_watch.pushed {
            return None;
        }
        let s = tr();
        let theme = cx.theme();
        Some(
            h_flex()
                .flex_none()
                .h(px(32.))
                .px_3()
                .gap_2()
                .items_center()
                .border_b_1()
                .border_color(theme.border)
                .bg(theme.info.opacity(0.12))
                .text_sm()
                .child(
                    Icon::new(Icons::GitCommitHorizontal)
                        .small()
                        .text_color(theme.info),
                )
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(s.pushed_banner.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(s.pushed_banner_desc.clone()),
                )
                .child(
                    Button::new("pushed-refresh")
                        .small()
                        .outline()
                        .icon(IconName::RefreshCw)
                        .label(s.refresh.clone())
                        .tooltip_with_action(s.refresh.clone(), &Refresh, Some("Review"))
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                )
                .into_any_element(),
        )
    }
}
