use gpui::{actions, AppContext, KeyBinding, Menu, MenuItem, SharedString};
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
    if cfg!(target_os = "macos") {
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.bind_keys([KeyBinding::new("cmd-right", Next, None)]);
        cx.bind_keys([KeyBinding::new("cmd-left", Previous, None)]);
    } else {
        cx.bind_keys([KeyBinding::new("ctrl-w", Quit, None)]);
        cx.bind_keys([KeyBinding::new("ctrl-right", Next, None)]);
        cx.bind_keys([KeyBinding::new("ctrl-left", Previous, None)]);
    }
    cx.bind_keys([KeyBinding::new("space", PlayPause, None)]);
    cx.set_menus(vec![Menu {
        name: SharedString::from("Muzak"),
        items: vec![MenuItem::action("Quit", Quit)],
    }]);
}

fn quit(_: &Quit, cx: &mut AppContext) {
    info!("Quitting...");
    cx.quit();
}

fn play_pause(_: &PlayPause, cx: &mut AppContext) {
    let state = cx.global::<PlaybackInfo>().playback_state.read(cx);
    let interface = cx.global::<GPUIPlaybackInterface>();
    match state {
        PlaybackState::Stopped => {
            interface.play();
        }
        PlaybackState::Playing => {
            interface.pause();
        }
        PlaybackState::Paused => {
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
