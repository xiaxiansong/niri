use std::fmt;
use std::sync::{Arc, Mutex};

use client::ClientId;
use insta::assert_snapshot;
use niri_config::{Action, Config};
use smithay::backend::renderer::element::{
    Id, PrimaryScanoutOutput, RenderElementPresentationState, RenderElementState,
    RenderElementStates,
};
use smithay::output::Output;
use smithay::reexports::wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use smithay::wayland::compositor::with_states;
use tracing::field::Field;
use tracing::subscriber::DefaultGuard;
use tracing::{Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::Registry;
use wayland_client::protocol::wl_surface::WlSurface;

use super::*;
use crate::layout::LayoutElement as _;
use crate::utils::with_toplevel_role;

/// Collects the error messages logged on the current thread.
#[derive(Clone, Default)]
struct ErrorCollector(Arc<Mutex<Vec<String>>>);

impl ErrorCollector {
    fn install() -> (Self, DefaultGuard) {
        let collector = Self::default();
        let subscriber = Registry::default().with(collector.clone());
        let guard = tracing::subscriber::set_default(subscriber);
        (collector, guard)
    }

    fn errors(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl<S: Subscriber> Layer<S> for ErrorCollector {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if *event.metadata().level() != Level::ERROR {
            return;
        }

        let mut message = String::new();
        event.record(&mut |field: &Field, value: &dyn fmt::Debug| {
            if field.name() == "message" {
                message = format!("{value:?}");
            }
        });
        self.0.lock().unwrap().push(message);
    }
}

fn set_up() -> (Fixture, ClientId) {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    (f, id)
}

fn set_up_with_config(config: &str) -> (Fixture, ClientId) {
    let mut f = Fixture::with_config(Config::parse_mem(config).unwrap());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    (f, id)
}

fn map_window(
    f: &mut Fixture,
    id: ClientId,
    title: &str,
    parent: Option<&XdgToplevel>,
) -> WlSurface {
    map_window_with_app_id(f, id, title, None, parent)
}

fn map_window_with_app_id(
    f: &mut Fixture,
    id: ClientId,
    title: &str,
    app_id: Option<&str>,
    parent: Option<&XdgToplevel>,
) -> WlSurface {
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.set_title(title);
    if let Some(app_id) = app_id {
        window.set_app_id(app_id);
    }
    if parent.is_some() {
        window.set_parent(parent);
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

fn window_id(f: &mut Fixture, t: &str) -> u64 {
    let mut windows = f.niri().layout.windows();
    let (_, mapped) = windows.find(|(_, mapped)| title(mapped) == t).unwrap();
    mapped.id().get()
}

fn format_windows(f: &mut Fixture) -> String {
    let layout = &f.niri().layout;
    let focused = layout.focus().map(|mapped| mapped.id());

    let mut lines = Vec::new();
    layout.with_windows(|mapped, _, ws_id, _| {
        lines.push(format!(
            "{}: {:?}, floating: {}, workspace: {}, focused: {}",
            title(mapped),
            mapped.scratchpad_state(),
            mapped.is_floating(),
            ws_id.is_some(),
            Some(mapped.id()) == focused,
        ));
    });
    lines.sort();
    lines.join("\n")
}

#[test]
fn hidden_window_keeps_committing() {
    let (errors, _guard) = ErrorCollector::install();
    let (mut f, id) = set_up();
    let surface = map_window(&mut f, id, "a", None);
    let _ = f.client(id).window(&surface).recent_configures();

    f.niri_state()
        .do_action(Action::MoveWindowToScratchpad, false);
    f.double_roundtrip(id);
    assert_snapshot!(
        format_windows(&mut f),
        @"a: Hidden, floating: true, workspace: false, focused: false"
    );

    // Hiding drops the Activated state. The size request stays the same (the tiled size), and
    // the bounds are left as is.
    let window = f.client(id).window(&surface);
    assert_snapshot!(
        window.format_recent_configures(),
        @"size: 936 × 1048, bounds: 1888 × 1048, states: []"
    );

    // The client acks and commits with a new size while hidden.
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let window = f.client(id).window(&surface);
    assert_snapshot!(window.format_recent_configures(), @"");
    let size = f.niri().layout.windows().next().unwrap().1.size();
    assert_eq!((size.w, size.h), (200, 200));

    // Showing requests 60% of the working area as the first floating size.
    f.niri_state()
        .do_action(Action::ScratchpadShow(None), false);
    f.double_roundtrip(id);
    assert_snapshot!(
        format_windows(&mut f),
        @"a: Shown, floating: true, workspace: true, focused: true"
    );
    let window = f.client(id).window(&surface);
    assert_snapshot!(
        window.format_recent_configures(),
        @"size: 1152 × 648, bounds: 1920 × 1080, states: [Activated]"
    );

    assert_eq!(errors.errors(), Vec::<String>::new());
}

#[test]
fn unmap_hidden_window() {
    let (errors, _guard) = ErrorCollector::install();
    let (mut f, id) = set_up();
    let surface = map_window(&mut f, id, "a", None);

    f.niri_state()
        .do_action(Action::MoveWindowToScratchpad, false);
    f.double_roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_null();
    window.commit();
    f.double_roundtrip(id);

    assert_snapshot!(format_windows(&mut f), @"");
    assert_eq!(f.niri().layout.scratchpad_hidden_windows().count(), 0);

    // Mapping again shows the window in the scratchpad. Unmapping discards the toplevel
    // attributes, so set the title again.
    let window = f.client(id).window(&surface);
    window.set_title("a");
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    assert_snapshot!(
        format_windows(&mut f),
        @"a: Shown, floating: true, workspace: true, focused: true"
    );

    assert_eq!(errors.errors(), Vec::<String>::new());
}

#[test]
fn new_dialog_of_hidden_window_shows_it() {
    let (errors, _guard) = ErrorCollector::install();
    let (mut f, id) = set_up();
    let surface = map_window(&mut f, id, "parent", None);
    let parent = f.client(id).window(&surface).xdg_toplevel.clone();

    f.niri_state()
        .do_action(Action::MoveWindowToScratchpad, false);
    f.double_roundtrip(id);

    map_window(&mut f, id, "child", Some(&parent));

    // Dialogs open floating. The parent is shown and activated first, so the dialog gets
    // focused next to it.
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    child: None, floating: true, workspace: true, focused: true
    parent: Shown, floating: true, workspace: true, focused: false
    "
    );

    assert_eq!(errors.errors(), Vec::<String>::new());
}

/// Updates the primary scanout output as if the window was rendered on output 1.
fn render_window_on_output_1(f: &mut Fixture) {
    let output = f.niri_output(1);
    let niri = f.niri();
    let surface = niri
        .layout
        .windows()
        .next()
        .unwrap()
        .1
        .toplevel()
        .wl_surface();
    let state = RenderElementState {
        visible_area: 1,
        presentation_state: RenderElementPresentationState::Rendering { reason: None },
        needs_capture: false,
    };
    let states = RenderElementStates {
        states: [(Id::from_wayland_resource(surface), state)].into(),
    };
    niri.update_primary_scanout_output(&output, &states);
}

fn primary_scanout_output(f: &mut Fixture) -> String {
    let niri = f.niri();
    let surface = niri
        .layout
        .windows()
        .next()
        .unwrap()
        .1
        .toplevel()
        .wl_surface();
    let output = with_states(surface, |states| {
        let output = states.data_map.get::<Mutex<PrimaryScanoutOutput>>()?;
        output.lock().unwrap().current_output()
    });
    format!("{:?}", output.map(|output| output.name()))
}

#[test]
fn hiding_clears_primary_scanout_output() {
    let (mut f, id) = set_up();
    map_window(&mut f, id, "a", None);

    render_window_on_output_1(&mut f);
    assert_snapshot!(primary_scanout_output(&mut f), @r#"Some("headless-1")"#);

    // Cleared right away, without waiting for a redraw.
    f.niri_state()
        .do_action(Action::MoveWindowToScratchpad, false);
    assert_snapshot!(primary_scanout_output(&mut f), @"None");
}

#[test]
fn redraw_clears_primary_scanout_output_of_hidden_window() {
    let (mut f, id) = set_up();
    map_window(&mut f, id, "a", None);

    render_window_on_output_1(&mut f);
    assert_snapshot!(primary_scanout_output(&mut f), @r#"Some("headless-1")"#);

    // Hide through the layout directly to skip the hiding side effects.
    f.niri().layout.move_to_scratchpad(None).unwrap();
    assert_snapshot!(primary_scanout_output(&mut f), @r#"Some("headless-1")"#);

    // Render output 1 without the window.
    let output = f.niri_output(1);
    f.niri()
        .update_primary_scanout_output(&output, &RenderElementStates::default());
    assert_snapshot!(primary_scanout_output(&mut f), @"None");
}

fn check_ipc(f: &mut Fixture) -> String {
    let (from_events, from_layout) = f.niri_state().ipc_windows_from_events_and_layout();
    assert_eq!(
        serde_json::to_value(&from_events).unwrap(),
        serde_json::to_value(&from_layout).unwrap(),
    );

    let win = &from_events[0];
    format!(
        "in scratchpad: {}, hidden: {}, workspace: {}",
        win.is_in_scratchpad,
        win.is_scratchpad_hidden,
        win.workspace_id.is_some(),
    )
}

#[test]
fn ipc_window_state_follows_scratchpad() {
    let (mut f, id) = set_up();
    map_window(&mut f, id, "a", None);
    assert_snapshot!(
        check_ipc(&mut f),
        @"in scratchpad: false, hidden: false, workspace: true"
    );

    f.niri_state()
        .do_action(Action::MoveWindowToScratchpad, false);
    f.double_roundtrip(id);
    assert_snapshot!(
        check_ipc(&mut f),
        @"in scratchpad: true, hidden: true, workspace: false"
    );

    f.niri_state()
        .do_action(Action::ScratchpadShow(None), false);
    f.double_roundtrip(id);
    assert_snapshot!(
        check_ipc(&mut f),
        @"in scratchpad: true, hidden: false, workspace: true"
    );

    // Only the scratchpad fields change here: the window stays floating on the same workspace.
    f.niri_state()
        .do_action(Action::RemoveWindowFromScratchpad, false);
    f.double_roundtrip(id);
    assert_snapshot!(
        check_ipc(&mut f),
        @"in scratchpad: false, hidden: false, workspace: true"
    );
}

#[test]
fn focus_window_previous_skips_hidden_windows() {
    let (mut f, id) = set_up();
    map_window(&mut f, id, "a", None);
    map_window(&mut f, id, "b", None);
    map_window(&mut f, id, "c", None);

    // C has the most recent focus timestamp.
    f.niri_state()
        .do_action(Action::MoveWindowToScratchpad, false);
    f.double_roundtrip(id);
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    a: None, floating: false, workspace: true, focused: false
    b: None, floating: false, workspace: true, focused: true
    c: Hidden, floating: true, workspace: false, focused: false
    "
    );

    f.niri_state().do_action(Action::FocusWindowPrevious, false);
    f.double_roundtrip(id);
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    a: None, floating: false, workspace: true, focused: true
    b: None, floating: false, workspace: true, focused: false
    c: Hidden, floating: true, workspace: false, focused: false
    "
    );
}

#[cfg(feature = "dbus")]
#[test]
fn screencast_window_list_skips_hidden_windows() {
    use crate::dbus::gnome_shell_introspect::{IntrospectToNiri, NiriToIntrospect};

    let (mut f, id) = set_up();
    map_window(&mut f, id, "a", None);
    map_window(&mut f, id, "b", None);

    f.niri_state()
        .do_action(Action::MoveWindowToScratchpad, false);
    f.double_roundtrip(id);

    let a = window_id(&mut f, "a");
    let b = window_id(&mut f, "b");

    let (tx, rx) = async_channel::unbounded();
    f.niri_state()
        .on_introspect_msg(&tx, IntrospectToNiri::GetWindows);
    let NiriToIntrospect::Windows(windows) = rx.try_recv().unwrap();
    assert!(windows.contains_key(&a));
    assert!(!windows.contains_key(&b));
}

fn output_events(f: &mut Fixture, id: ClientId, surface: &WlSurface) -> String {
    let events: Vec<_> = f
        .client(id)
        .state
        .recent_output_events(surface)
        .map(|event| event.to_string())
        .collect();
    events.join(", ")
}

fn is_on_active_workspace(f: &mut Fixture, t: &str) -> bool {
    let ws = f.niri().layout.active_workspace().unwrap();
    ws.windows().any(|mapped| title(mapped) == t)
}

#[test]
fn following_window_stays_on_output() {
    let (mut f, id) = set_up();
    let surface = map_window(&mut f, id, "a", None);

    // Hiding leaves the output, and showing enters it again.
    f.niri_state()
        .do_action(Action::MoveWindowToScratchpad, false);
    f.double_roundtrip(id);
    f.niri_state()
        .do_action(Action::ScratchpadShow(None), false);
    f.double_roundtrip(id);
    assert_snapshot!(
        output_events(&mut f, id, &surface),
        @"enter headless-1, leave headless-1, enter headless-1"
    );

    // Following to another workspace on the same output doesn't leave and enter the output.
    f.niri_state().do_action(Action::FocusWorkspaceDown, false);
    f.double_roundtrip(id);
    assert!(is_on_active_workspace(&mut f, "a"));
    f.niri_state().do_action(Action::FocusWorkspaceUp, false);
    f.double_roundtrip(id);
    assert!(is_on_active_workspace(&mut f, "a"));
    assert_snapshot!(output_events(&mut f, id, &surface), @"");
}

#[test]
fn following_window_moves_to_other_output_once() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));
    let id = f.add_client();
    let surface = map_window(&mut f, id, "a", None);

    f.niri_state()
        .do_action(Action::MoveWindowToScratchpad, false);
    f.double_roundtrip(id);
    f.niri_state()
        .do_action(Action::ScratchpadShow(None), false);
    f.double_roundtrip(id);
    let _ = output_events(&mut f, id, &surface);

    // The workspace moves to the other output, then the window follows to its active workspace.
    let output = f.niri_output(1);
    f.niri().remove_output(&output);
    f.double_roundtrip(id);
    assert!(is_on_active_workspace(&mut f, "a"));
    assert_snapshot!(
        output_events(&mut f, id, &surface),
        @"leave headless-1, enter headless-2"
    );
}

fn is_on_active_workspace_of(f: &mut Fixture, output: &Output, t: &str) -> bool {
    let mon = f.niri().layout.monitor_for_output(output).unwrap();
    let ws = mon.active_workspace_ref();
    ws.windows().any(|mapped| title(mapped) == t)
}

fn hide_and_show(f: &mut Fixture, id: ClientId) {
    f.niri_state()
        .do_action(Action::MoveWindowToScratchpad, false);
    f.double_roundtrip(id);
    f.niri_state()
        .do_action(Action::ScratchpadShow(None), false);
    f.double_roundtrip(id);
}

#[test]
fn shown_window_stays_on_output_when_workspace_moves_to_other_monitor() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));
    let output1 = f.niri_output(1);
    let id = f.add_client();
    let surface = map_window(&mut f, id, "a", None);
    hide_and_show(&mut f, id);
    let _ = output_events(&mut f, id, &surface);

    f.niri_state()
        .do_action(Action::MoveWorkspaceToMonitorRight, false);
    f.double_roundtrip(id);
    assert!(is_on_active_workspace_of(&mut f, &output1, "a"));
    assert_snapshot!(output_events(&mut f, id, &surface), @"");
}

#[test]
fn shown_window_stays_on_output_when_workspace_goes_back_to_its_output() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));
    let output1 = f.niri_output(1);
    let output2 = f.niri_output(2);
    let id = f.add_client();
    f.niri_focus_output(2);
    let _ = map_window(&mut f, id, "b", None);
    let surface = map_window(&mut f, id, "a", None);

    // Show the window on the workspace from output 2 after it moved to output 1.
    f.niri_state()
        .do_action(Action::MoveWindowToScratchpad, false);
    f.double_roundtrip(id);
    f.niri().remove_output(&output2);
    f.niri_state().do_action(Action::FocusWorkspaceUp, false);
    f.niri_state()
        .do_action(Action::ScratchpadShow(None), false);
    f.double_roundtrip(id);
    assert!(is_on_active_workspace_of(&mut f, &output1, "a"));
    assert!(is_on_active_workspace_of(&mut f, &output1, "b"));
    let _ = output_events(&mut f, id, &surface);

    // The workspace goes back to output 2, but the window stays.
    f.niri().add_output(output2.clone(), None, false);
    f.double_roundtrip(id);
    assert!(is_on_active_workspace_of(&mut f, &output1, "a"));
    assert!(is_on_active_workspace_of(&mut f, &output2, "b"));
    assert_snapshot!(output_events(&mut f, id, &surface), @"");
}

const DROPDOWN_RULE: &str = r#"
window-rule {
    match app-id="^dd$"
    open-in-scratchpad true
}
"#;

#[test]
fn open_in_scratchpad_rule() {
    let (errors, _guard) = ErrorCollector::install();
    let (mut f, id) = set_up_with_config(DROPDOWN_RULE);
    map_window(&mut f, id, "other", None);

    let dd = map_window_with_app_id(&mut f, id, "dd", Some("dd"), None);
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    dd: Hidden, floating: true, workspace: false, focused: false
    other: None, floating: false, workspace: true, focused: true
    "
    );
    // The window never gets the focus, so it never gets Activated.
    let window = f.client(id).window(&dd);
    assert!(!window.format_recent_configures().contains("Activated"));
    // It was never visible, so there is no hiding animation.
    let dd_win = dd_window(&mut f);
    assert!(f.niri().layout.hiding_window_output(&dd_win).is_none());

    // The first show uses 60% of the working area.
    f.niri_state()
        .do_action(Action::ScratchpadShow(None), false);
    f.double_roundtrip(id);
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    dd: Shown, floating: true, workspace: true, focused: true
    other: None, floating: false, workspace: true, focused: false
    "
    );
    let window = f.client(id).window(&dd);
    assert_snapshot!(
        window.format_recent_configures(),
        @"size: 1152 × 648, bounds: 1920 × 1080, states: [Activated]"
    );

    assert_eq!(errors.errors(), Vec::<String>::new());
}

fn dd_window(f: &mut Fixture) -> smithay::desktop::Window {
    let mut windows = f.niri().layout.windows();
    let (_, mapped) = windows.find(|(_, mapped)| title(mapped) == "dd").unwrap();
    mapped.window.clone()
}

fn rules_of(f: &mut Fixture, t: &str) -> String {
    let mut windows = f.niri().layout.windows();
    let (_, mapped) = windows.find(|(_, mapped)| title(mapped) == t).unwrap();
    format!(
        "{:?}, opacity: {:?}, open-in-scratchpad: {:?}",
        mapped.scratchpad_state(),
        mapped.rules().opacity,
        mapped.rules().open_in_scratchpad,
    )
}

#[test]
fn rules_do_not_flip_scratchpad_state() {
    let (errors, _guard) = ErrorCollector::install();
    // The matcher on is-in-scratchpad turns open-in-scratchpad off while in the scratchpad and on
    // again outside of it. If recomputing the rules acted on it, the window would go back and
    // forth.
    let (mut f, id) = set_up_with_config(
        r#"
window-rule {
    match app-id="^dd$"
    open-in-scratchpad true
}
window-rule {
    match app-id="^dd$" is-in-scratchpad=true
    open-in-scratchpad false
    opacity 0.5
}
window-rule {
    match app-id="^dd$" is-in-scratchpad=false
    open-in-scratchpad true
}
"#,
    );
    map_window_with_app_id(&mut f, id, "dd", Some("dd"), None);
    f.double_roundtrip(id);
    assert_snapshot!(
        rules_of(&mut f, "dd"),
        @"Hidden, opacity: Some(0.5), open-in-scratchpad: Some(false)"
    );

    f.niri_state()
        .do_action(Action::ScratchpadShow(None), false);
    f.double_roundtrip(id);
    assert_snapshot!(
        rules_of(&mut f, "dd"),
        @"Shown, opacity: Some(0.5), open-in-scratchpad: Some(false)"
    );

    f.niri_state()
        .do_action(Action::RemoveWindowFromScratchpad, false);
    f.double_roundtrip(id);
    assert_snapshot!(
        rules_of(&mut f, "dd"),
        @"None, opacity: None, open-in-scratchpad: Some(true)"
    );

    // And still not hidden after another round of refreshing.
    f.double_roundtrip(id);
    assert_snapshot!(
        rules_of(&mut f, "dd"),
        @"None, opacity: None, open-in-scratchpad: Some(true)"
    );

    assert_eq!(errors.errors(), Vec::<String>::new());
}

#[test]
fn is_in_scratchpad_matcher() {
    let (mut f, id) = set_up_with_config(
        r#"
window-rule {
    match is-in-scratchpad=true
    opacity 0.5
}
"#,
    );
    map_window(&mut f, id, "a", None);
    f.double_roundtrip(id);
    // The window matched as a regular one when it opened.
    assert_snapshot!(rules_of(&mut f, "a"), @"None, opacity: None, open-in-scratchpad: None");

    f.niri_state()
        .do_action(Action::MoveWindowToScratchpad, false);
    f.double_roundtrip(id);
    assert_snapshot!(rules_of(&mut f, "a"), @"Hidden, opacity: Some(0.5), open-in-scratchpad: None");

    f.niri_state()
        .do_action(Action::ScratchpadShow(None), false);
    f.double_roundtrip(id);
    assert_snapshot!(rules_of(&mut f, "a"), @"Shown, opacity: Some(0.5), open-in-scratchpad: None");

    f.niri_state()
        .do_action(Action::RemoveWindowFromScratchpad, false);
    f.double_roundtrip(id);
    assert_snapshot!(rules_of(&mut f, "a"), @"None, opacity: None, open-in-scratchpad: None");
}

#[test]
fn open_in_scratchpad_ignored_for_dialogs() {
    let (mut f, id) = set_up_with_config(DROPDOWN_RULE);
    let parent = map_window(&mut f, id, "parent", None);
    let parent = f.client(id).window(&parent).xdg_toplevel.clone();

    map_window_with_app_id(&mut f, id, "dd", Some("dd"), Some(&parent));
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    dd: None, floating: true, workspace: true, focused: true
    parent: None, floating: false, workspace: true, focused: false
    "
    );
}

#[test]
fn open_in_scratchpad_ipc_starts_hidden() {
    let (mut f, id) = set_up_with_config(DROPDOWN_RULE);
    map_window_with_app_id(&mut f, id, "dd", Some("dd"), None);
    f.double_roundtrip(id);
    assert_snapshot!(
        check_ipc(&mut f),
        @"in scratchpad: true, hidden: true, workspace: false"
    );
}

#[test]
fn open_in_scratchpad_floating_window_shows_where_it_opened() {
    let (mut f, id) = set_up_with_config(
        r#"
window-rule {
    match app-id="^dd$"
    open-in-scratchpad true
    open-floating true
    default-floating-position x=20 y=30
}
"#,
    );
    let dd = map_window_with_app_id(&mut f, id, "dd", Some("dd"), None);
    f.double_roundtrip(id);

    f.niri_state()
        .do_action(Action::ScratchpadShow(None), false);
    f.double_roundtrip(id);

    let pos = {
        let ws = f.niri().layout.active_workspace().unwrap();
        let (_, pos) = ws
            .floating()
            .tiles_with_offsets()
            .find(|(tile, _)| title(tile.window()) == "dd")
            .unwrap();
        (pos.x, pos.y)
    };
    assert_eq!(pos, (20., 30.));
    // The window keeps the size it opened with, instead of the default for the first show.
    let window = f.client(id).window(&dd);
    assert_snapshot!(
        window.format_recent_configures(),
        @r"
    size: 0 × 0, bounds: 1920 × 1080, states: []
    size: 100 × 100, bounds: 1920 × 1080, states: []
    size: 100 × 100, bounds: 1920 × 1080, states: [Activated]
    "
    );
}

#[test]
fn open_in_scratchpad_does_not_activate_output() {
    let mut f = Fixture::with_config(
        Config::parse_mem(
            r#"
window-rule {
    match app-id="^dd$"
    open-in-scratchpad true
    open-on-output "headless-2"
    open-focused true
}
"#,
        )
        .unwrap(),
    );
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    f.niri_focus_output(1);
    let id = f.add_client();
    map_window(&mut f, id, "other", None);

    // The window is added on output 2, but hiding it must leave the focus and the active output
    // where they were.
    map_window_with_app_id(&mut f, id, "dd", Some("dd"), None);
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    dd: Hidden, floating: true, workspace: false, focused: false
    other: None, floating: false, workspace: true, focused: true
    "
    );
    let output = f.niri().layout.active_output().unwrap().name();
    assert_eq!(output, "headless-1");
}

/// Unmaps the window and maps it again, like an application hiding to the tray and coming back.
///
/// Unmapping discards the toplevel attributes, so they are set again.
fn remap(f: &mut Fixture, id: ClientId, surface: &WlSurface, title: &str, app_id: &str) {
    let window = f.client(id).window(surface);
    window.attach_null();
    window.commit();
    f.double_roundtrip(id);

    let window = f.client(id).window(surface);
    window.set_title(title);
    window.set_app_id(app_id);
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(surface);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.double_roundtrip(id);
}

fn do_action(f: &mut Fixture, id: ClientId, action: Action) {
    f.niri_state().do_action(action, false);
    f.double_roundtrip(id);
}

#[test]
fn remapped_window_is_shown_in_scratchpad() {
    let (errors, _guard) = ErrorCollector::install();
    let (mut f, id) = set_up();
    map_window(&mut f, id, "other", None);
    let surface = map_window(&mut f, id, "a", None);

    // A hidden window comes back shown, on the workspace that is active now.
    do_action(&mut f, id, Action::MoveWindowToScratchpad);
    do_action(&mut f, id, Action::FocusWorkspaceDown);
    f.niri_complete_animations();
    remap(&mut f, id, &surface, "a", "");
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    a: Shown, floating: true, workspace: true, focused: true
    other: None, floating: false, workspace: true, focused: false
    "
    );
    assert!(is_on_active_workspace(&mut f, "a"));
    assert_snapshot!(
        check_ipc_all(&mut f),
        @"a: in scratchpad: true, hidden: false, workspace: true; other: in scratchpad: false, hidden: false, workspace: true"
    );

    // So does a shown one.
    remap(&mut f, id, &surface, "a", "");
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    a: Shown, floating: true, workspace: true, focused: true
    other: None, floating: false, workspace: true, focused: false
    "
    );

    // A regular window stays regular.
    do_action(&mut f, id, Action::RemoveWindowFromScratchpad);
    remap(&mut f, id, &surface, "a", "");
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    a: None, floating: false, workspace: true, focused: true
    other: None, floating: false, workspace: true, focused: false
    "
    );

    assert_eq!(errors.errors(), Vec::<String>::new());
}

fn check_ipc_all(f: &mut Fixture) -> String {
    let (from_events, from_layout) = f.niri_state().ipc_windows_from_events_and_layout();
    assert_eq!(
        serde_json::to_value(&from_events).unwrap(),
        serde_json::to_value(&from_layout).unwrap(),
    );

    let mut windows: Vec<_> = from_events
        .iter()
        .map(|win| {
            format!(
                "{}: in scratchpad: {}, hidden: {}, workspace: {}",
                win.title.as_deref().unwrap(),
                win.is_in_scratchpad,
                win.is_scratchpad_hidden,
                win.workspace_id.is_some(),
            )
        })
        .collect();
    windows.sort();
    windows.join("; ")
}

#[test]
fn remapped_window_respects_open_focused() {
    let (mut f, id) = set_up_with_config(
        r#"
window-rule {
    match app-id="^dd$"
    open-focused false
}
"#,
    );
    map_window(&mut f, id, "other", None);
    let surface = map_window_with_app_id(&mut f, id, "a", Some("dd"), None);
    let a = window_id(&mut f, "a");
    do_action(&mut f, id, Action::MoveWindowToScratchpadById(a));

    remap(&mut f, id, &surface, "a", "dd");
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    a: Shown, floating: true, workspace: true, focused: false
    other: None, floating: false, workspace: true, focused: true
    "
    );
}

#[test]
fn remapped_window_and_open_in_scratchpad_rule() {
    // dd: the rule hides new windows. nd: the rule opts out.
    let (mut f, id) = set_up_with_config(
        r#"
window-rule {
    match app-id="^dd$"
    open-in-scratchpad true
}
window-rule {
    match app-id="^nd$"
    open-in-scratchpad false
}
"#,
    );
    let dd = map_window_with_app_id(&mut f, id, "dd", Some("dd"), None);
    let nd = map_window_with_app_id(&mut f, id, "nd", Some("nd"), None);
    do_action(&mut f, id, Action::MoveWindowToScratchpad);
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    dd: Hidden, floating: true, workspace: false, focused: false
    nd: Hidden, floating: true, workspace: false, focused: false
    "
    );

    // In the scratchpad before: shown, even though the rule hides, unless the rule says no.
    remap(&mut f, id, &dd, "dd", "dd");
    remap(&mut f, id, &nd, "nd", "nd");
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    dd: Shown, floating: true, workspace: true, focused: false
    nd: None, floating: false, workspace: true, focused: true
    "
    );

    // Not in the scratchpad before: the rule applies as for a new window.
    let dd_id = window_id(&mut f, "dd");
    do_action(&mut f, id, Action::RemoveWindowFromScratchpadById(dd_id));
    remap(&mut f, id, &dd, "dd", "dd");
    assert_snapshot!(
        format_windows(&mut f),
        @r"
    dd: Hidden, floating: true, workspace: false, focused: false
    nd: None, floating: false, workspace: true, focused: true
    "
    );
}

#[test]
fn remapped_window_without_outputs_stays_hidden() {
    let (errors, _guard) = ErrorCollector::install();
    let (mut f, id) = set_up();
    let surface = map_window(&mut f, id, "a", None);
    do_action(&mut f, id, Action::MoveWindowToScratchpad);

    let window = f.client(id).window(&surface);
    window.attach_null();
    window.commit();
    f.double_roundtrip(id);

    let output = f.niri_output(1);
    f.niri().remove_output(&output);

    let window = f.client(id).window(&surface);
    window.set_title("a");
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    assert_snapshot!(
        format_windows(&mut f),
        @"a: Hidden, floating: true, workspace: false, focused: false"
    );

    // It can be shown once there is an output again.
    f.add_output(1, (1920, 1080));
    do_action(&mut f, id, Action::ScratchpadShow(None));
    assert_snapshot!(
        format_windows(&mut f),
        @"a: Shown, floating: true, workspace: true, focused: true"
    );

    assert_eq!(errors.errors(), Vec::<String>::new());
}
