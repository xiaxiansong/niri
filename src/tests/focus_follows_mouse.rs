use std::thread;
use std::time::Duration;

use client::ClientId;
use insta::assert_snapshot;
use niri_config::{Action, Config};
use smithay::utils::{Logical, Point};

use super::*;
use crate::utils::with_toplevel_role;

const DELAY_MS: u64 = 50;

fn set_up(delay: bool) -> (Fixture, ClientId) {
    let delay = if delay {
        format!("delay-ms={DELAY_MS}")
    } else {
        String::new()
    };
    // Three columns that fit on the screen, so that the view never needs to scroll.
    let config = format!(
        r#"
input {{
    focus-follows-mouse {delay}
}}
layout {{
    default-column-width {{ proportion 0.25; }}
}}
"#
    );
    let mut f = Fixture::with_config(Config::parse_mem(&config).unwrap());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    for title in ["a", "b", "c"] {
        let window = f.client(id).create_window();
        let surface = window.surface.clone();
        window.set_title(title);
        window.commit();
        f.roundtrip(id);

        let window = f.client(id).window(&surface);
        window.attach_new_buffer();
        window.set_size(400, 1000);
        window.ack_last_and_commit();
        f.double_roundtrip(id);
    }

    f.niri_state().do_action(Action::FocusColumnFirst, false);
    f.double_roundtrip(id);

    (f, id)
}

fn title(window: &smithay::desktop::Window) -> String {
    with_toplevel_role(window.toplevel().unwrap(), |role| {
        role.title.clone().unwrap_or_default()
    })
}

fn focused(f: &mut Fixture) -> String {
    let focus = f.niri().layout.focus().map(|mapped| title(&mapped.window));
    format!("{focus:?}")
}

/// Finds a point in the middle of the screen height over the window with this title.
fn point_over(f: &mut Fixture, target: &str) -> Point<f64, Logical> {
    let niri = f.niri();
    (0..1920)
        .step_by(10)
        .map(|x| Point::from((f64::from(x), 540.)))
        .find(|pos| {
            let contents = niri.contents_under(*pos);
            contents
                .window
                .is_some_and(|(window, _)| title(&window) == target)
        })
        .unwrap()
}

fn move_pointer(f: &mut Fixture, id: ClientId, target: &str) {
    let pos = point_over(f, target);

    let client = f.client(id);
    let manager = client.state.virtual_pointer_manager.as_ref().unwrap();
    let pointer = manager.create_virtual_pointer(None, &client.qh, ());
    pointer.motion_absolute(0, pos.x as u32, pos.y as u32, 1920, 1080);
    pointer.frame();
    pointer.destroy();
    f.double_roundtrip(id);
}

/// Lets the delay pass and runs the timers.
fn wait(f: &mut Fixture, id: ClientId) {
    thread::sleep(Duration::from_millis(DELAY_MS * 2));
    f.dispatch();
    f.double_roundtrip(id);
}

#[test]
fn without_delay_focuses_right_away() {
    let (mut f, id) = set_up(false);
    move_pointer(&mut f, id, "a");
    move_pointer(&mut f, id, "b");
    assert_snapshot!(focused(&mut f), @r#"Some("b")"#);
}

#[test]
fn delay_postpones_focus() {
    let (mut f, id) = set_up(true);
    move_pointer(&mut f, id, "a");
    move_pointer(&mut f, id, "b");
    assert_snapshot!(focused(&mut f), @r#"Some("a")"#);

    wait(&mut f, id);
    assert_snapshot!(focused(&mut f), @r#"Some("b")"#);
}

#[test]
fn passing_over_window_does_not_focus_it() {
    let (mut f, id) = set_up(true);
    move_pointer(&mut f, id, "a");
    move_pointer(&mut f, id, "b");
    move_pointer(&mut f, id, "a");
    wait(&mut f, id);
    assert_snapshot!(focused(&mut f), @r#"Some("a")"#);

    move_pointer(&mut f, id, "b");
    move_pointer(&mut f, id, "c");
    wait(&mut f, id);
    assert_snapshot!(focused(&mut f), @r#"Some("c")"#);
}

#[test]
fn focus_change_during_delay_is_kept() {
    let (mut f, id) = set_up(true);
    move_pointer(&mut f, id, "a");
    move_pointer(&mut f, id, "b");

    // Without the delay, the pointer would have focused b first, and the focus action would have
    // moved the focus on to c. Keep it that way.
    f.niri_state().do_action(Action::FocusColumnLast, false);
    f.double_roundtrip(id);
    wait(&mut f, id);
    assert_snapshot!(focused(&mut f), @r#"Some("c")"#);
}

#[test]
fn window_moving_away_during_delay_is_not_focused() {
    let (mut f, id) = set_up(true);
    move_pointer(&mut f, id, "a");
    move_pointer(&mut f, id, "b");

    // a and b swap places, so a ends up under the pointer without the pointer moving.
    f.niri_state().do_action(Action::MoveColumnRight, false);
    f.double_roundtrip(id);
    f.niri_complete_animations();
    wait(&mut f, id);
    assert_snapshot!(focused(&mut f), @r#"Some("a")"#);
}
