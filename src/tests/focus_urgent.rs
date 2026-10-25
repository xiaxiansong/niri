use client::ClientId;
use insta::assert_snapshot;
use niri_config::{Action, Config};

use super::*;
use crate::utils::with_toplevel_role;

fn set_up() -> (Fixture, ClientId) {
    let mut f = Fixture::with_config(Config::default());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    (f, id)
}

fn map_window(f: &mut Fixture, id: ClientId, title: &str) {
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.set_title(title);
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(100, 100);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
}

fn window_id(f: &mut Fixture, title: &str) -> u64 {
    let mut windows = f.niri().layout.windows();
    let (_, mapped) = windows
        .find(|(_, mapped)| {
            with_toplevel_role(mapped.toplevel(), |role| {
                role.title.as_deref() == Some(title)
            })
        })
        .unwrap();
    mapped.id().get()
}

fn action(f: &mut Fixture, id: ClientId, action: Action) {
    f.niri_state().do_action(action, false);
    f.double_roundtrip(id);
}

fn set_urgent(f: &mut Fixture, id: ClientId, title: &str) {
    let window = window_id(f, title);
    action(f, id, Action::SetWindowUrgent(window));
}

fn state(f: &mut Fixture) -> String {
    let layout = &f.niri().layout;
    let title = layout.focus().map(|mapped| {
        with_toplevel_role(mapped.toplevel(), |role| {
            role.title.clone().unwrap_or_default()
        })
    });
    let ws_idx = layout
        .active_monitor_ref()
        .map(|mon| mon.active_workspace_idx());
    let mut urgent: Vec<_> = layout
        .windows()
        .filter(|(_, mapped)| mapped.is_urgent())
        .map(|(_, mapped)| {
            with_toplevel_role(mapped.toplevel(), |role| {
                role.title.clone().unwrap_or_default()
            })
        })
        .collect();
    urgent.sort();
    format!("focus: {title:?}, workspace: {ws_idx:?}, urgent: {urgent:?}")
}

#[test]
fn without_urgent_windows_does_nothing() {
    let (mut f, id) = set_up();
    map_window(&mut f, id, "a");
    map_window(&mut f, id, "b");
    action(&mut f, id, Action::FocusUrgentWindow);
    assert_snapshot!(state(&mut f), @r#"focus: Some("b"), workspace: Some(0), urgent: []"#);
}

#[test]
fn focuses_most_recently_urgent_first() {
    let (mut f, id) = set_up();
    map_window(&mut f, id, "a");
    map_window(&mut f, id, "b");
    map_window(&mut f, id, "c");
    set_urgent(&mut f, id, "b");
    set_urgent(&mut f, id, "a");
    assert_snapshot!(state(&mut f), @r#"focus: Some("c"), workspace: Some(0), urgent: ["a", "b"]"#);

    // Focusing a window clears its urgency, so the next one is the window that was marked
    // urgent before it.
    action(&mut f, id, Action::FocusUrgentWindow);
    assert_snapshot!(state(&mut f), @r#"focus: Some("a"), workspace: Some(0), urgent: ["b"]"#);
    action(&mut f, id, Action::FocusUrgentWindow);
    assert_snapshot!(state(&mut f), @r#"focus: Some("b"), workspace: Some(0), urgent: []"#);
    action(&mut f, id, Action::FocusUrgentWindow);
    assert_snapshot!(state(&mut f), @r#"focus: Some("b"), workspace: Some(0), urgent: []"#);
}

#[test]
fn marking_urgent_again_moves_window_first() {
    let (mut f, id) = set_up();
    map_window(&mut f, id, "a");
    map_window(&mut f, id, "b");
    map_window(&mut f, id, "c");
    set_urgent(&mut f, id, "a");
    set_urgent(&mut f, id, "b");
    // a asks for attention again, so it's now the most recent one.
    set_urgent(&mut f, id, "a");

    action(&mut f, id, Action::FocusUrgentWindow);
    assert_snapshot!(state(&mut f), @r#"focus: Some("a"), workspace: Some(0), urgent: ["b"]"#);
}

#[test]
fn switches_to_workspace_of_urgent_window() {
    let (mut f, id) = set_up();
    map_window(&mut f, id, "a");
    action(&mut f, id, Action::FocusWorkspaceDown);
    map_window(&mut f, id, "b");
    set_urgent(&mut f, id, "a");
    assert_snapshot!(state(&mut f), @r#"focus: Some("b"), workspace: Some(1), urgent: ["a"]"#);

    action(&mut f, id, Action::FocusUrgentWindow);
    assert_snapshot!(state(&mut f), @r#"focus: Some("a"), workspace: Some(0), urgent: []"#);
}
