use gpui::{actions, AppContext, KeyBinding, Menu, MenuItem};
use tracing::{debug, info};

use crate::playback::{interface::GPUIPlaybackInterface, thread::PlaybackState};

use super::models::PlaybackInfo;

actions!(muzak, [Quit, PlayPause, Next, Previous]);

pub fn register_actions(cx: &mut AppContext) {
    debug!("registering actions");
    cx.on_action(quit);
    cx.on_action(play_pause);
    cx.on_action(next);
    cx.on_action(previous);
    debug!("actions: {:?}", cx.all_action_names());
    debug!("action available: {:?}", cx.is_action_available(&Quit));
    cx.bind_keys([KeyBinding::new("ctrl-w", Quit, None)]);
    cx.bind_keys([KeyBinding::new("space", PlayPause, None)]);
    cx.set_menus(vec![Menu {
        name: "Muzak",
        items: vec![MenuItem::action("Quit", Quit)],
    }]);
}

fn quit(_: &Quit, cx: &mut AppContext) {
    info!("Quitting...");
    cx.quit();
}

fn play_pause(_: &PlayPause, cx: &mut AppContext) {
    let state = cx.global::<PlaybackInfo>().playback_state.read(cx);
    match state {
        PlaybackState::Stopped => (),
        PlaybackState::Playing => {
            let interface = cx.global::<GPUIPlaybackInterface>();
            interface.pause();
        }
        PlaybackState::Paused => {
            let interface = cx.global::<GPUIPlaybackInterface>();
            interface.play();
        }
    }
}

fn next(_: &Next, cx: &mut AppContext) {
    let interface = cx.global::<GPUIPlaybackInterface>();
    interface.next();
}

fn previous(_: &Previous, cx: &mut AppContext) {
    let interface = cx.global::<GPUIPlaybackInterface>();
    interface.previous();
}