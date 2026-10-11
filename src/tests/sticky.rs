use client::ClientId;
use insta::assert_snapshot;
use niri_config::{Action, Config};
use wayland_client::protocol::wl_surface::WlSurface;

use super::*;
use crate::layout::LayoutElement as _;
use crate::utils::with_toplevel_role;

fn set_up_with_config(config: &str) -> (Fixture, ClientId) {
    let mut f = Fixture::with_config(Config::parse_mem(config).unwrap());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    (f, id)
}

fn map_window(f: &mut Fixture, id: ClientId, title: &str, app_id: Option<&str>) -> WlSurface {
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.set_title(title);
    if let Some(app_id) = app_id {
        window.set_app_id(app_id);
    }
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(100, 100);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    surface
}

fn title(mapped: &crate::window::Mapped) -> String {
    with_toplevel_role(mapped.toplevel(), |role| {
        role.title.clone().unwrap_or_default()
    })
}

/// Windows with whether they are sticky, floating, on the active workspace, and focused.
fn format_windows(f: &mut Fixture) -> String {
    let layout = &f.niri().layout;
    let focused = layout.focus().map(|mapped| mapped.id());
    let active_ws = layout.active_workspace().map(|ws| ws.id());

    let mut lines = Vec::new();
    layout.with_windows(|mapped, _, ws_id, _| {
        lines.push(format!(
            "{}: sticky: {}, floating: {}, active workspace: {}, focused: {}",
            title(mapped),
            mapped.is_sticky(),
            mapped.is_floating(),
            ws_id.is_some() && ws_id == active_ws,
            Some(mapped.id()) == focused,
        ));
    });
    lines.sort();
    lines.join("\n")
}

#[test]
fn open_sticky_rule() {
    let (mut f, id) = set_up_with_config(
        r#"
window-rule {
    match app-id="^pip$"
    open-sticky true
}
"#,
    );
    map_window(&mut f, id, "other", None);
    let pip = map_window(&mut f, id, "pip", Some("pip"));
    assert_snapshot!(format_windows(&mut f), @"
    other: sticky: false, floating: false, active workspace: true, focused: false
    pip: sticky: true, floating: true, active workspace: true, focused: true
    ");

    // The window opened floating right away.
    let window = f.client(id).window(&pip);
    assert_snapshot!(window.format_recent_configures(), @"
    size: 0 × 0, bounds: 1920 × 1080, states: []
    size: 100 × 100, bounds: 1920 × 1080, states: [Activated]
    ");

    f.niri_state().do_action(Action::FocusWorkspaceDown, false);
    f.double_roundtrip(id);
    assert_snapshot!(format_windows(&mut f), @"
    other: sticky: false, floating: false, active workspace: false, focused: false
    pip: sticky: true, floating: true, active workspace: true, focused: true
    ");
}

#[test]
fn is_sticky_matcher() {
    let (mut f, id) = set_up_with_config(
        r#"
window-rule {
    match is-sticky=true
    opacity 0.5
}
"#,
    );
    map_window(&mut f, id, "a", None);

    let opacity = |f: &mut Fixture| {
        let mut windows = f.niri().layout.windows();
        let (_, mapped) = windows.next().unwrap();
        format!("{:?}", mapped.rules().opacity)
    };
    assert_snapshot!(opacity(&mut f), @"None");

    f.niri_state().do_action(Action::ToggleWindowSticky, false);
    f.double_roundtrip(id);
    assert_snapshot!(opacity(&mut f), @"Some(0.5)");

    f.niri_state().do_action(Action::ToggleWindowSticky, false);
    f.double_roundtrip(id);
    assert_snapshot!(opacity(&mut f), @"None");
}

#[test]
fn toggle_window_sticky_bind_and_ipc() {
    let (mut f, id) = set_up_with_config(
        r#"
binds {
    Mod+P { toggle-window-sticky; }
}
"#,
    );
    let action = f.niri().config.borrow().binds.0[0].action.clone();
    assert_eq!(action, Action::ToggleWindowSticky);

    map_window(&mut f, id, "a", None);
    map_window(&mut f, id, "b", None);

    let a_id = {
        let mut windows = f.niri().layout.windows();
        let (_, mapped) = windows.find(|(_, mapped)| title(mapped) == "a").unwrap();
        mapped.id().get()
    };
    let action = Action::from(niri_ipc::Action::ToggleWindowSticky { id: Some(a_id) });
    assert_eq!(action, Action::ToggleWindowStickyById(a_id));
    f.niri_state().do_action(action, false);
    f.double_roundtrip(id);
    assert_snapshot!(format_windows(&mut f), @"
    a: sticky: true, floating: true, active workspace: true, focused: false
    b: sticky: false, floating: false, active workspace: true, focused: true
    ");

    let (from_events, from_layout) = f.niri_state().ipc_windows_from_events_and_layout();
    assert_eq!(
        serde_json::to_value(&from_events).unwrap(),
        serde_json::to_value(&from_layout).unwrap(),
    );
    let mut sticky: Vec<_> = from_events
        .iter()
        .map(|win| (win.title.clone().unwrap(), win.is_sticky))
        .collect();
    sticky.sort();
    assert_snapshot!(format!("{sticky:?}"), @r#"[("a", true), ("b", false)]"#);
}
