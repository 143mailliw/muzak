use gpui::*;
use tracing::error;

use crate::{
    services::mmb::lastfm::{client::LastFMClient, LASTFM_API_KEY, LASTFM_API_SECRET},
    ui::{
        constants::FONT_AWESOME_BRANDS,
        models::{LastFMState, MMBSList, Models},
        theme::Theme,
    },
};

pub struct LastFM {
    mmbs: Model<MMBSList>,
    state: Model<LastFMState>,
    name: Option<SharedString>,
}

impl LastFM {
    pub fn new<V: 'static>(cx: &mut ViewContext<V>) -> View<Self> {
        cx.new_view(|cx| {
            let models = cx.global::<Models>();
            let mmbs = models.mmbs.clone();
            let state = models.lastfm.clone();

            cx.observe(&state, |this: &mut LastFM, m, cx| {
                this.name = match m.read(cx) {
                    LastFMState::Connected(session) => Some(session.name.clone().into()),
                    _ => None,
                }
            })
            .detach();

            LastFM {
                mmbs,
                name: match state.read(cx) {
                    LastFMState::Connected(session) => Some(session.name.clone().into()),
                    _ => None,
                },
                state,
            }
        })
    }
}

impl Render for LastFM {
    fn render(&mut self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let state = self.state.clone();

        div()
            .flex()
            .text_sm()
            .px(px(12.0))
            .pb(px(6.0))
            .pt(px(5.0))
            .text_color(theme.text_secondary)
            .bg(theme.window_button)
            .id("lastfm-button")
            .hover(|this| this.bg(theme.window_button_hover))
            .active(|this| this.bg(theme.window_button_active))
            .on_mouse_down(MouseButton::Left, |_, cx| {
                cx.prevent_default();
                cx.stop_propagation();
            })
            .child(
                div()
                    .font_family(FONT_AWESOME_BRANDS)
                    .mr(px(8.0))
                    .pt(px(3.0))
                    .text_size(px(11.0))
                    .h_full()
                    .child(""),
            )
            .child(
                div().child(match self.state.read(cx) {
                    LastFMState::Disconnected => "Sign in".into_any_element(),
                    LastFMState::AwaitingFinalization(_) => {
                        "Click to confirm sign in".into_any_element()
                    }
                    LastFMState::Connected(_) => self
                        .name
                        .clone()
                        .unwrap_or(SharedString::new_static("Connected"))
                        .into_any_element(),
                }),
            )
            .on_click(move |_, cx| {
                let state = state.clone();
                let read = state.read(cx).clone();

                match read {
                    LastFMState::Disconnected => get_token(cx, state),
                    LastFMState::AwaitingFinalization(token) => confirm(cx, state, token),
                    LastFMState::Connected(_) => (),
                }
            })
    }
}

fn get_token(cx: &mut WindowContext<'_>, state: Model<LastFMState>) {
    cx.spawn(|mut cx| async move {
        let mut client = LastFMClient::new(
            LASTFM_API_KEY.unwrap().to_string(),
            LASTFM_API_SECRET.unwrap(),
        );

        if let Ok(token) = client.get_token().await {
            let path = format!(
                "http://last.fm/api/auth/?api_key={}&token={}",
                LASTFM_API_KEY.unwrap(),
                token
            );
            if open::that(&path).is_err() {
                error!(
                    "Failed to open web browser to {}; you'll need to navigate to it manually.",
                    path
                );
            }
            state
                .update(&mut cx, move |m, cx| {
                    *m = LastFMState::AwaitingFinalization(token);
                    cx.notify();
                })
                .expect("failed to update lastfm state");
        } else {
            error!("error getting token");
        }
    })
    .detach();
}

fn confirm(cx: &mut WindowContext<'_>, state: Model<LastFMState>, token: String) {
    cx.spawn(|mut cx| async move {
        let mut client = LastFMClient::new(
            LASTFM_API_KEY.unwrap().to_string(),
            LASTFM_API_SECRET.unwrap(),
        );

        if let Ok(session) = client.get_session(token).await {
            state
                .update(&mut cx, move |_, cx| {
                    cx.emit(session);
                })
                .expect("failed to emit session event");
        } else {
            error!("error getting session")
        }
    })
    .detach();
}