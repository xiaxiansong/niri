use std::fmt;
use std::sync::{Arc, Mutex};

use client::ClientId;
use insta::assert_snapshot;
use niri_config::Action;
use smithay::backend::renderer::element::{
    Id, PrimaryScanoutOutput, RenderElementPresentationState, RenderElementState,
    RenderElementStates,
};
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

fn map_window(
    f: &mut Fixture,
    id: ClientId,
    title: &str,
    parent: Option<&XdgToplevel>,
) -> WlSurface {
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.set_title(title);
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

    // Mapping again gives a regular window. Unmapping discards the toplevel attributes, so
    // set the title again.
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
        @"a: None, floating: false, workspace: true, focused: true"
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
