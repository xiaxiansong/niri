use client::ClientId;
use insta::assert_snapshot;
use niri_config::{Action, Config};
use wayland_client::protocol::wl_surface::WlSurface;

use super::*;
use crate::utils::with_toplevel_role;

fn set_up(focus_previous: bool) -> (Fixture, ClientId) {
    let flag = if focus_previous {
        "focus-previous-window-on-close"
    } else {
        ""
    };
    // Without the debounce, focusing a window updates its focus timestamp right away.
    let config = format!(
        r#"
input {{
    {flag}
}}
recent-windows {{
    debounce-ms 0
}}
"#
    );
    let mut f = Fixture::with_config(Config::parse_mem(&config).unwrap());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    (f, id)
}

fn map_window(f: &mut Fixture, id: ClientId, title: &str) -> WlSurface {
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

    surface
}

fn focused(f: &mut Fixture) -> String {
    let layout = &f.niri().layout;
    let title = layout.focus().map(|mapped| {
        with_toplevel_role(mapped.toplevel(), |role| {
            role.title.clone().unwrap_or_default()
        })
    });
    let ws_idx = layout
        .active_monitor_ref()
        .map(|mon| mon.active_workspace_idx());
    format!("focus: {title:?}, workspace: {ws_idx:?}")
}

fn action(f: &mut Fixture, id: ClientId, action: Action) {
    f.niri_state().do_action(action, false);
    f.double_roundtrip(id);
}

fn destroy(f: &mut Fixture, id: ClientId, surface: &WlSurface) {
    let window = f.client(id).window(surface);
    window.xdg_toplevel.destroy();
    window.xdg_surface.destroy();
    f.double_roundtrip(id);
}

fn unmap(f: &mut Fixture, id: ClientId, surface: &WlSurface) {
    let window = f.client(id).window(surface);
    window.attach_null();
    window.commit();
    f.double_roundtrip(id);
}

/// Opens windows a, b and c in columns, focuses a and closes it.
fn close_first_column(focus_previous: bool) -> String {
    let (mut f, id) = set_up(focus_previous);
    let a = map_window(&mut f, id, "a");
    map_window(&mut f, id, "b");
    map_window(&mut f, id, "c");
    action(&mut f, id, Action::FocusColumnFirst);
    let before = focused(&mut f);
    destroy(&mut f, id, &a);
    format!("{before}\nafter close: {}", focused(&mut f))
}

#[test]
fn close_focuses_previous_window() {
    // Normally, the focus goes to the neighbor b. With the option, it goes back to c, which was
    // focused before a.
    assert_snapshot!(close_first_column(false), @r#"
    focus: Some("a"), workspace: Some(0)
    after close: focus: Some("b"), workspace: Some(0)
    "#);
    assert_snapshot!(close_first_column(true), @r#"
    focus: Some("a"), workspace: Some(0)
    after close: focus: Some("c"), workspace: Some(0)
    "#);
}

/// Opens a on the first workspace and b on the second, and closes b.
fn close_on_other_workspace(focus_previous: bool, unmap_instead: bool) -> String {
    let (mut f, id) = set_up(focus_previous);
    map_window(&mut f, id, "a");
    action(&mut f, id, Action::FocusWorkspaceDown);
    let b = map_window(&mut f, id, "b");
    let before = focused(&mut f);
    if unmap_instead {
        unmap(&mut f, id, &b);
    } else {
        destroy(&mut f, id, &b);
    }
    format!("{before}\nafter close: {}", focused(&mut f))
}

#[test]
fn close_switches_workspace() {
    assert_snapshot!(close_on_other_workspace(false, false), @r#"
    focus: Some("b"), workspace: Some(1)
    after close: focus: None, workspace: Some(1)
    "#);
    assert_snapshot!(close_on_other_workspace(true, false), @r#"
    focus: Some("b"), workspace: Some(1)
    after close: focus: Some("a"), workspace: Some(0)
    "#);
    // Unmapping counts as closing too.
    assert_snapshot!(close_on_other_workspace(true, true), @r#"
    focus: Some("b"), workspace: Some(1)
    after close: focus: Some("a"), workspace: Some(0)
    "#);
}

#[test]
fn closing_unfocused_window_keeps_focus() {
    let (mut f, id) = set_up(true);
    let a = map_window(&mut f, id, "a");
    map_window(&mut f, id, "b");
    map_window(&mut f, id, "c");
    action(&mut f, id, Action::FocusColumnFirst);
    action(&mut f, id, Action::FocusColumnRight);
    destroy(&mut f, id, &a);
    assert_snapshot!(focused(&mut f), @r#"focus: Some("b"), workspace: Some(0)"#);
}

#[test]
fn hidden_scratchpad_window_is_skipped() {
    let (mut f, id) = set_up(true);
    map_window(&mut f, id, "a");
    map_window(&mut f, id, "b");
    action(&mut f, id, Action::MoveWindowToScratchpad);
    let c = map_window(&mut f, id, "c");
    destroy(&mut f, id, &c);
    assert_snapshot!(focused(&mut f), @r#"focus: Some("a"), workspace: Some(0)"#);
}
