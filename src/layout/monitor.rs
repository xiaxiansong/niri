use std::cmp::min;
use std::iter::zip;
use std::rc::Rc;
use std::time::Duration;

use niri_config::{CornerRadius, LayoutPart};
use smithay::backend::renderer::element::utils::{
    CropRenderElement, Relocate, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::Color32F;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Rectangle, Size};

use super::insert_hint_element::{InsertHintElement, InsertHintRenderElement};
use super::scrolling::{Column, ColumnWidth};
use super::tile::Tile;
use super::workspace::{
    compute_working_area, resize_edges_for_tile, OutputId, TakenScratchpadTile, Workspace,
    WorkspaceAddWindowTarget, WorkspaceId, WorkspaceRenderElement,
};
use super::{compute_overview_zoom, ActivateWindow, HitType, LayoutElement, Options};
use crate::animation::{Animation, Clock};
use crate::input::swipe_tracker::SwipeTracker;
use crate::layout::RenderLayer;
use crate::niri_render_elements;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::shadow::ShadowRenderElement;
use crate::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};
use crate::render_helpers::xray::XrayPos;
use crate::render_helpers::RenderCtx;
use crate::rubber_band::RubberBand;
use crate::utils::transaction::Transaction;
use crate::utils::{
    output_size, round_logical_in_physical, round_logical_in_physical_max1, ResizeEdge,
};

/// Amount of touchpad movement to scroll the height of one workspace.
const WORKSPACE_GESTURE_MOVEMENT: f64 = 300.;

const WORKSPACE_GESTURE_RUBBER_BAND: RubberBand = RubberBand {
    stiffness: 0.5,
    limit: 0.05,
};

/// Amount of DnD edge scrolling to scroll the height of one workspace.
///
/// This constant is tied to the default dnd-edge-workspace-switch max-speed setting.
const WORKSPACE_DND_EDGE_SCROLL_MOVEMENT: f64 = 1500.;

#[derive(Debug)]
pub struct Monitor<W: LayoutElement> {
    /// Output for this monitor.
    pub(super) output: Output,
    /// Cached name of the output.
    output_name: String,
    /// Latest known scale for this output.
    scale: smithay::output::Scale,
    /// Latest known size for this output.
    view_size: Size<f64, Logical>,
    /// Latest known working area for this output.
    ///
    /// Not rounded to physical pixels.
    // FIXME: since this is used for things like DnD scrolling edges in the overview, ideally this
    // should only consider overlay and top layer-shell surfaces. However, Smithay doesn't easily
    // let you do this at the moment.
    working_area: Rectangle<f64, Logical>,
    // Must always contain at least one.
    pub(super) workspaces: Vec<Workspace<W>>,
    /// Index of the currently active workspace.
    pub(super) active_workspace_idx: usize,
    /// ID of the previously active workspace.
    pub(super) previous_workspace_id: Option<WorkspaceId>,
    /// In-progress switch between workspaces.
    pub(super) workspace_switch: Option<WorkspaceSwitch>,
    /// Indication where an interactively-moved window is about to be placed.
    pub(super) insert_hint: Option<InsertHint>,
    /// Buffers for darkening workspaces during a skipping workspace switch, one per workspace
    /// since every element needs its own ID for damage tracking.
    skip_dim_buffers: Vec<SolidColorBuffer>,
    /// Where to draw the darkening buffers in use, in output coordinates.
    pub(super) skip_dim_geos: Vec<Rectangle<f64, Logical>>,
    /// Insert hint element for rendering.
    insert_hint_element: InsertHintElement,
    /// Location to render the insert hint element.
    insert_hint_render_loc: Option<InsertHintRenderLoc>,
    /// Whether the overview is open.
    pub(super) overview_open: bool,
    /// Progress of the overview zoom animation, 1 is fully in overview.
    overview_progress: Option<OverviewProgress>,
    /// Clock for driving animations.
    pub(super) clock: Clock,
    /// Configurable properties of the layout as received from the parent layout.
    pub(super) base_options: Rc<Options>,
    /// Configurable properties of the layout.
    pub(super) options: Rc<Options>,
    /// Layout config overrides for this monitor.
    layout_config: Option<niri_config::LayoutPart>,
}

#[derive(Debug)]
pub enum WorkspaceSwitch {
    Animation(Animation),
    Skip(WorkspaceSwitchSkip),
    Gesture(WorkspaceSwitchGesture),
}

#[derive(Debug)]
pub struct WorkspaceSwitchGesture {
    /// Index of the workspace where the gesture was started.
    center_idx: usize,
    /// Fractional workspace index where the gesture was started.
    ///
    /// Can differ from center_idx when starting a gesture in the middle between workspaces, for
    /// example by "catching" an animation.
    start_idx: f64,
    /// Current, fractional workspace index.
    pub(super) current_idx: f64,
    /// Animation for the extra offset to the current position.
    ///
    /// For example, if there's a workspace switch during a DnD scroll.
    animation: Option<Animation>,
    tracker: SwipeTracker,
    /// Whether the gesture is controlled by the touchpad.
    is_touchpad: bool,
    /// Whether the gesture is clamped to +-1 workspace around the center.
    is_clamped: bool,

    // If this gesture is for drag-and-drop scrolling, this is the last event's unadjusted
    // timestamp.
    dnd_last_event_time: Option<Duration>,
    // Time when the drag-and-drop scroll delta became non-zero, used for debouncing.
    //
    // If `None` then the scroll delta is currently zero.
    dnd_nonzero_start_time: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InsertPosition {
    NewColumn(usize),
    InColumn(usize, usize),
    Floating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InsertWorkspace {
    Existing(WorkspaceId),
    NewAt(usize),
}

#[derive(Debug)]
pub(super) struct InsertHint {
    pub workspace: InsertWorkspace,
    pub position: InsertPosition,
    pub corner_radius: CornerRadius,
}

#[derive(Debug, Clone, Copy)]
struct InsertHintRenderLoc {
    workspace: InsertWorkspace,
    location: Point<f64, Logical>,
}

#[derive(Debug)]
pub(super) enum OverviewProgress {
    Animation(Animation),
    Value(f64),
}

/// Where to put a newly added window.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum MonitorAddWindowTarget<'a, W: LayoutElement> {
    /// No particular preference.
    #[default]
    Auto,
    /// On this workspace.
    Workspace {
        /// Id of the target workspace.
        id: WorkspaceId,
        /// Override where the window will open as a new column.
        column_idx: Option<usize>,
    },
    /// Next to this existing window.
    NextTo(&'a W::Id),
}

impl<'a, W: LayoutElement> Copy for MonitorAddWindowTarget<'a, W> {}

impl<'a, W: LayoutElement> Clone for MonitorAddWindowTarget<'a, W> {
    fn clone(&self) -> Self {
        *self
    }
}

niri_render_elements! {
    MonitorInnerRenderElement<R> => {
        Workspace = CropRenderElement<WorkspaceRenderElement<R>>,
        InsertHint = CropRenderElement<InsertHintRenderElement>,
        UncroppedInsertHint = InsertHintRenderElement,
        Shadow = ShadowRenderElement,
        SolidColor = SolidColorRenderElement,
    }
}

pub type MonitorRenderElement<R> =
    RelocateRenderElement<RescaleRenderElement<MonitorInnerRenderElement<R>>>;

/// A workspace with its render geometry, and the part of it that isn't covered by others if it's
/// cut off.
pub type WorkspaceGeoAndClip<'a, W> = (
    &'a Workspace<W>,
    Rectangle<f64, Logical>,
    Option<Rectangle<f64, Logical>>,
);

/// How much to darken a workspace that is fully moved out, during a skipping workspace switch.
const SKIP_DIM: f32 = 0.35;

/// Where workspaces that aren't a part of a skipping workspace switch are, far out of view.
const SKIP_OUT_OF_VIEW_Y: f64 = 1e6;

/// A workspace switch where the workspaces in between aren't shown.
///
/// Workspaces that aren't next to each other still move like they were: the visible workspaces
/// and the one we're switching to form a strip that scrolls as a whole. This also lets an
/// interrupted switch continue from wherever it was, without any jumps.
///
/// When an interrupted switch goes to a workspace that belongs between two visible ones, there's
/// no room for it in the strip. Then the workspaces on one side slide out of view, and the target
/// comes in from under them. So that the workspaces never overlap, each one has a visible range
/// that cuts off the part that's covered.
#[derive(Debug)]
pub struct WorkspaceSwitchSkip {
    /// Progress of the switch, from 0 to 1.
    anim: Animation,
    /// Workspaces that are shown at some point during the switch.
    items: Vec<SkipItem>,
    /// Index of the workspace we're switching to.
    target_idx: f64,
}

/// A value that changes linearly with the progress of a skipping workspace switch.
#[derive(Debug, Clone, Copy)]
struct SkipLine {
    from: f64,
    delta: f64,
}

/// A workspace taking part in a skipping workspace switch.
///
/// All values are in workspace heights (including the gap) from where the active workspace is at
/// rest.
#[derive(Debug, Clone, Copy)]
struct SkipItem {
    id: WorkspaceId,
    /// Position of the top of the workspace.
    y: SkipLine,
    /// Top of the visible range, or negative infinity if not cut off at the top.
    top: SkipLine,
    /// Bottom of the visible range, or positive infinity if not cut off at the bottom.
    bottom: SkipLine,
}

/// Current position of a [`SkipItem`].
#[derive(Debug, Clone, Copy)]
struct SkipPos {
    /// Index of the workspace.
    idx: usize,
    y: f64,
    top: f64,
    bottom: f64,
}

impl SkipLine {
    fn new(from: f64, delta: f64) -> Self {
        Self { from, delta }
    }

    fn at(self, progress: f64) -> f64 {
        // Infinities stay infinite: delta is always finite.
        self.from + self.delta * progress
    }
}

impl SkipPos {
    /// Visible range of the workspace.
    fn visible(&self) -> (f64, f64) {
        (self.y.max(self.top), (self.y + 1.).min(self.bottom))
    }

    /// How much of the screen the visible range takes up.
    fn on_screen(&self) -> f64 {
        let (top, bottom) = self.visible();
        (bottom.min(1.) - top.max(0.)).max(0.)
    }
}

impl WorkspaceSwitchSkip {
    /// How much to darken a workspace at the position: the further it is, the darker.
    fn dim_alpha(y: f64) -> f32 {
        SKIP_DIM * y.abs().min(1.) as f32
    }
}

impl WorkspaceSwitch {
    /// Current workspace render index.
    ///
    /// Returns `None` for a skipping switch, where the workspaces aren't laid out in a strip by
    /// index; see [`Monitor::workspace_render_idx()`] for that.
    pub fn current_idx(&self) -> Option<f64> {
        match self {
            WorkspaceSwitch::Animation(anim) => Some(anim.value()),
            WorkspaceSwitch::Skip(_) => None,
            WorkspaceSwitch::Gesture(gesture) => Some(
                gesture.current_idx + gesture.animation.as_ref().map_or(0., |anim| anim.value()),
            ),
        }
    }

    pub fn target_idx(&self) -> f64 {
        match self {
            WorkspaceSwitch::Animation(anim) => anim.to(),
            WorkspaceSwitch::Skip(skip) => skip.target_idx,
            WorkspaceSwitch::Gesture(gesture) => gesture.current_idx,
        }
    }

    pub fn offset(&mut self, delta: isize) {
        match self {
            WorkspaceSwitch::Animation(anim) => anim.offset(delta as f64),
            // The items are tracked by workspace ID, so only the target index needs adjusting.
            WorkspaceSwitch::Skip(skip) => skip.target_idx += delta as f64,
            WorkspaceSwitch::Gesture(gesture) => {
                if delta >= 0 {
                    gesture.center_idx += delta as usize;
                } else {
                    gesture.center_idx -= (-delta) as usize;
                }
                gesture.start_idx += delta as f64;
                gesture.current_idx += delta as f64;
            }
        }
    }

    fn is_animation_ongoing(&self) -> bool {
        match self {
            WorkspaceSwitch::Animation(_) | WorkspaceSwitch::Skip(_) => true,
            WorkspaceSwitch::Gesture(gesture) => gesture.animation.is_some(),
        }
    }
}

impl WorkspaceSwitchGesture {
    fn min_max(&self, workspace_count: usize) -> (f64, f64) {
        if self.is_clamped {
            let min = self.center_idx.saturating_sub(1) as f64;
            let max = (self.center_idx + 1).min(workspace_count - 1) as f64;
            (min, max)
        } else {
            (0., (workspace_count - 1) as f64)
        }
    }

    fn animate_from(&mut self, from: f64, clock: Clock, config: niri_config::Animation) {
        let current = self.animation.as_ref().map_or(0., Animation::value);
        self.animation = Some(Animation::new(clock, from + current, 0., 0., config));
    }
}

impl InsertWorkspace {
    fn existing_id(self) -> Option<WorkspaceId> {
        match self {
            InsertWorkspace::Existing(id) => Some(id),
            InsertWorkspace::NewAt(_) => None,
        }
    }
}

impl OverviewProgress {
    pub fn value(&self) -> f64 {
        match self {
            OverviewProgress::Animation(anim) => anim.value(),
            OverviewProgress::Value(v) => *v,
        }
    }

    pub fn clamped_value(&self) -> f64 {
        match self {
            OverviewProgress::Animation(anim) => anim.clamped_value(),
            OverviewProgress::Value(v) => *v,
        }
    }
}

impl From<&super::OverviewProgress> for OverviewProgress {
    fn from(value: &super::OverviewProgress) -> Self {
        match value {
            super::OverviewProgress::Animation(anim) => Self::Animation(anim.clone()),
            super::OverviewProgress::Gesture(gesture) => Self::Value(gesture.value),
            super::OverviewProgress::Open => Self::Value(1.),
        }
    }
}

impl<W: LayoutElement> Monitor<W> {
    pub fn new(
        output: Output,
        mut workspaces: Vec<Workspace<W>>,
        ws_id_to_activate: Option<WorkspaceId>,
        clock: Clock,
        base_options: Rc<Options>,
        layout_config: Option<LayoutPart>,
    ) -> Self {
        let options =
            Rc::new(Options::clone(&base_options).with_merged_layout(layout_config.as_ref()));

        let scale = output.current_scale();
        let view_size = output_size(&output);
        let working_area = compute_working_area(&output);

        // Prepare the workspaces: set output, empty first, empty last.
        let mut active_workspace_idx = 0;

        for (idx, ws) in workspaces.iter_mut().enumerate() {
            assert!(ws.has_windows_or_name());

            ws.set_output(Some(output.clone()));
            ws.update_config(options.clone());

            if ws_id_to_activate.is_some_and(|id| ws.id() == id) {
                active_workspace_idx = idx;
            }
        }

        if options.layout.empty_workspace_above_first && !workspaces.is_empty() {
            let ws = Workspace::new(output.clone(), clock.clone(), options.clone());
            workspaces.insert(0, ws);
            active_workspace_idx += 1;
        }

        let ws = Workspace::new(output.clone(), clock.clone(), options.clone());
        workspaces.push(ws);

        Self {
            output_name: output.name(),
            output,
            scale,
            view_size,
            working_area,
            workspaces,
            active_workspace_idx,
            previous_workspace_id: None,
            insert_hint: None,
            skip_dim_buffers: Vec::new(),
            skip_dim_geos: Vec::new(),
            insert_hint_element: InsertHintElement::new(options.layout.insert_hint),
            insert_hint_render_loc: None,
            overview_open: false,
            overview_progress: None,
            workspace_switch: None,
            clock,
            base_options,
            options,
            layout_config,
        }
    }

    pub fn into_workspaces(mut self) -> Vec<Workspace<W>> {
        self.workspaces.retain(|ws| ws.has_windows_or_name());

        for ws in &mut self.workspaces {
            ws.set_output(None);
        }

        self.workspaces
    }

    pub fn output(&self) -> &Output {
        &self.output
    }

    pub fn output_name(&self) -> &String {
        &self.output_name
    }

    pub fn active_workspace_idx(&self) -> usize {
        self.active_workspace_idx
    }

    pub fn active_workspace_ref(&self) -> &Workspace<W> {
        &self.workspaces[self.active_workspace_idx]
    }

    pub fn find_named_workspace(&self, workspace_name: &str) -> Option<&Workspace<W>> {
        self.workspaces.iter().find(|ws| {
            ws.name
                .as_ref()
                .is_some_and(|name| name.eq_ignore_ascii_case(workspace_name))
        })
    }

    pub fn find_named_workspace_index(&self, workspace_name: &str) -> Option<usize> {
        self.workspaces.iter().position(|ws| {
            ws.name
                .as_ref()
                .is_some_and(|name| name.eq_ignore_ascii_case(workspace_name))
        })
    }

    pub fn active_workspace(&mut self) -> &mut Workspace<W> {
        &mut self.workspaces[self.active_workspace_idx]
    }

    pub fn idx_of_ws(&self, id: WorkspaceId) -> Option<usize> {
        self.workspaces.iter().position(|ws| ws.id() == id)
    }

    pub fn has_ws(&self, id: WorkspaceId) -> bool {
        self.idx_of_ws(id).is_some()
    }

    pub fn windows(&self) -> impl Iterator<Item = &W> {
        self.workspaces.iter().flat_map(|ws| ws.windows())
    }

    pub fn has_window(&self, window: &W::Id) -> bool {
        self.windows().any(|win| win.id() == window)
    }

    pub fn add_workspace_at(&mut self, idx: usize) {
        let ws = Workspace::new(
            self.output.clone(),
            self.clock.clone(),
            self.options.clone(),
        );

        self.workspaces.insert(idx, ws);
        if idx <= self.active_workspace_idx {
            self.active_workspace_idx += 1;
        }

        if let Some(switch) = &mut self.workspace_switch {
            if idx as f64 <= switch.target_idx() {
                switch.offset(1);
            }
        }
    }

    pub fn add_workspace_top(&mut self) {
        self.add_workspace_at(0);
    }

    pub fn add_workspace_bottom(&mut self) {
        self.add_workspace_at(self.workspaces.len());
    }

    pub fn activate_workspace(&mut self, idx: usize) {
        self.activate_workspace_with_anim_config(idx, None);
    }

    pub fn activate_workspace_with_anim_config(
        &mut self,
        idx: usize,
        config: Option<niri_config::Animation>,
    ) {
        // FIXME: also compute and use current velocity.
        let current_idx = self.workspace_render_idx();

        if self.active_workspace_idx != idx {
            self.previous_workspace_id = Some(self.workspaces[self.active_workspace_idx].id());
        }

        let prev_active_idx = self.active_workspace_idx;

        let config = config.unwrap_or(self.options.animations.workspace_switch.0);

        // Must be computed before changing the active workspace.
        let skip = if prev_active_idx != idx {
            self.start_workspace_switch_skip(idx, config)
        } else {
            None
        };

        self.active_workspace_idx = idx;

        match &mut self.workspace_switch {
            // During a DnD scroll, we want to visually animate even if idx matches the active idx.
            Some(WorkspaceSwitch::Gesture(gesture)) if gesture.dnd_last_event_time.is_some() => {
                gesture.center_idx = idx;

                // Adjust start_idx to make current_idx point at idx.
                let current_pos = gesture.current_idx - gesture.start_idx;
                gesture.start_idx = idx as f64 - current_pos;
                let prev_current_idx = gesture.current_idx;
                gesture.current_idx = idx as f64;

                let current_idx_delta = gesture.current_idx - prev_current_idx;
                gesture.animate_from(-current_idx_delta, self.clock.clone(), config);
            }
            _ => {
                // Don't animate if nothing changed.
                if prev_active_idx != idx {
                    self.workspace_switch = Some(match skip {
                        Some(skip) => WorkspaceSwitch::Skip(skip),
                        None => WorkspaceSwitch::Animation(Animation::new(
                            self.clock.clone(),
                            current_idx,
                            idx as f64,
                            0.,
                            config,
                        )),
                    });
                }
            }
        }

        self.scratchpad_follow();
    }

    pub(super) fn resolve_add_window_target<'a>(
        &mut self,
        target: MonitorAddWindowTarget<'a, W>,
    ) -> (usize, WorkspaceAddWindowTarget<'a, W>) {
        match target {
            MonitorAddWindowTarget::Auto => {
                (self.active_workspace_idx, WorkspaceAddWindowTarget::Auto)
            }
            MonitorAddWindowTarget::Workspace { id, column_idx } => {
                let idx = self.idx_of_ws(id).unwrap();
                let target = if let Some(column_idx) = column_idx {
                    WorkspaceAddWindowTarget::NewColumnAt(column_idx)
                } else {
                    WorkspaceAddWindowTarget::Auto
                };
                (idx, target)
            }
            MonitorAddWindowTarget::NextTo(win_id) => {
                let idx = self
                    .workspaces
                    .iter_mut()
                    .position(|ws| ws.has_window(win_id))
                    .unwrap();
                (idx, WorkspaceAddWindowTarget::NextTo(win_id))
            }
        }
    }

    pub fn add_window(
        &mut self,
        window: W,
        target: MonitorAddWindowTarget<W>,
        activate: ActivateWindow,
        width: ColumnWidth,
        is_full_width: bool,
        is_floating: bool,
    ) {
        // Currently, everything a workspace sets on a Tile is the same across all workspaces of a
        // monitor. So we can use any workspace, not necessarily the exact target workspace.
        let tile = self.workspaces[0].make_tile(window);

        self.add_tile(
            tile,
            target,
            activate,
            true,
            width,
            is_full_width,
            is_floating,
            None,
        );
    }

    pub fn add_column(
        &mut self,
        mut workspace_idx: usize,
        column: Column<W>,
        activate: bool,
        anim: Option<niri_config::Animation>,
    ) {
        let workspace = &mut self.workspaces[workspace_idx];

        workspace.add_column(column, activate, anim);

        // After adding a new window, workspace becomes this output's own.
        if workspace.name().is_none() {
            workspace.original_outputs = vec![OutputId::new(&self.output)];
        }

        if workspace_idx == self.workspaces.len() - 1 {
            self.add_workspace_bottom();
        }
        if self.options.layout.empty_workspace_above_first && workspace_idx == 0 {
            self.add_workspace_top();
            workspace_idx += 1;
        }

        if activate {
            let id = self.workspaces[workspace_idx]
                .active_window()
                .unwrap()
                .id()
                .clone();
            self.activate_workspace(workspace_idx);
            // Shown scratchpad windows following to this workspace shouldn't take the focus.
            self.active_workspace().activate_window(&id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_tile(
        &mut self,
        tile: Tile<W>,
        target: MonitorAddWindowTarget<W>,
        activate: ActivateWindow,
        // FIXME: Refactor ActivateWindow enum to make this better.
        allow_to_activate_workspace: bool,
        width: ColumnWidth,
        is_full_width: bool,
        is_floating: bool,
        anim: Option<niri_config::Animation>,
    ) {
        let (workspace_idx, target) = self.resolve_add_window_target(target);

        let id = tile.window().id().clone();
        let workspace = &mut self.workspaces[workspace_idx];

        workspace.add_tile(
            tile,
            target,
            activate,
            width,
            is_full_width,
            is_floating,
            anim,
        );

        // After adding a new window, workspace becomes this output's own.
        if workspace.name().is_none() {
            workspace.original_outputs = vec![OutputId::new(&self.output)];
        }

        let workspace_idx = self.ensure_empty_workspaces_after_add(workspace_idx);

        if allow_to_activate_workspace && activate.map_smart(|| false) {
            self.activate_workspace(workspace_idx);
            // Shown scratchpad windows following to this workspace shouldn't take the focus.
            self.active_workspace().activate_window(&id);
        }
    }

    /// Adds empty workspaces around the workspace at `idx` that just got a window, if needed.
    ///
    /// Returns the new index of that workspace.
    pub(super) fn ensure_empty_workspaces_after_add(&mut self, mut idx: usize) -> usize {
        if idx == self.workspaces.len() - 1 {
            // Insert a new empty workspace.
            self.add_workspace_bottom();
        }

        if self.options.layout.empty_workspace_above_first && idx == 0 {
            self.add_workspace_top();
            idx += 1;
        }

        idx
    }

    /// Moves shown scratchpad windows from inactive workspaces to the active workspace.
    ///
    /// Outside the overview, this also moves their hiding and closing animations.
    pub fn scratchpad_follow(&mut self) {
        let active_idx = self.active_workspace_idx;
        let active_id = self.workspaces[active_idx].id();

        // In the overview, the windows can be seen moving between workspaces, so animate that.
        let animate = self.overview_progress.is_some() && self.workspace_switch.is_some();

        // Outside the overview, the windows are pinned to the screen, and so are their hiding and
        // closing animations. Keep those on the screen too. In the overview, leave them where
        // they are rather than have them jump between workspaces.
        let move_animations = self.overview_progress.is_none();

        let mut tiles = Vec::new();
        let mut moves = Vec::new();
        let mut animations = Vec::new();
        for (idx, ws) in self.workspaces.iter_mut().enumerate() {
            if idx != active_idx {
                let taken = ws.take_shown_scratchpad_tiles();
                if animate {
                    for taken in &taken {
                        moves.push((taken.tile.window().id().clone(), ws.id(), taken.pos));
                    }
                }
                tiles.extend(taken);

                if move_animations {
                    animations.push(ws.take_scratchpad_animations());
                }
            }
        }

        self.add_shown_scratchpad_tiles(tiles);

        let new_idx = self.idx_of_ws(active_id).unwrap();
        for animations in animations {
            self.workspaces[new_idx].add_scratchpad_animations(animations);
        }

        // Animate like Monitor::move_to_workspace(), following the workspace switch.
        let config = self.options.animations.workspace_switch.0;
        let ws_height_with_gap = self.workspace_size_with_gap(1.).h;
        for (id, source_id, mut old_pos) in moves {
            // There's a workspace switch, so the source workspace wasn't cleaned up.
            let source_idx = self.idx_of_ws(source_id).unwrap();
            old_pos.y += ws_height_with_gap * (source_idx as f64 - new_idx as f64);

            let (tile, render_pos) = self.workspaces[new_idx]
                .tiles_with_render_positions_mut(false)
                .find(|(tile, _)| tile.window().id() == &id)
                .unwrap();
            let new_pos = render_pos - tile.render_offset();
            tile.animate_move_from_with_config(old_pos - new_pos, config);
            tile.set_anim_y_between_workspaces();
        }
    }

    /// Adds shown scratchpad tiles taken from workspaces on this output to the active workspace.
    pub(super) fn add_shown_scratchpad_tiles(&mut self, tiles: Vec<TakenScratchpadTile<W>>) {
        if tiles.is_empty() {
            return;
        }

        let active_idx = self.active_workspace_idx;

        // Floating tiles are added on top, so add them from bottom to top to keep the order.
        let ws = &mut self.workspaces[active_idx];
        for taken in tiles.into_iter().rev() {
            ws.add_scratchpad_tile(taken.tile, taken.was_focused, false);
        }

        self.ensure_empty_workspaces_after_add(active_idx);

        // Without a workspace switch, nothing else will clean up the workspaces we emptied.
        if self.workspace_switch.is_none() {
            self.clean_up_workspaces();
        }
    }

    pub fn add_tile_to_column(
        &mut self,
        workspace_idx: usize,
        column_idx: usize,
        tile_idx: Option<usize>,
        tile: Tile<W>,
        activate: bool,
        // FIXME: Refactor ActivateWindow enum to make this better.
        allow_to_activate_workspace: bool,
    ) {
        let id = tile.window().id().clone();
        let workspace = &mut self.workspaces[workspace_idx];

        workspace.add_tile_to_column(column_idx, tile_idx, tile, activate);

        // After adding a new window, workspace becomes this output's own.
        if workspace.name().is_none() {
            workspace.original_outputs = vec![OutputId::new(&self.output)];
        }

        // Since we're adding window to an existing column, the workspace isn't empty, and
        // therefore cannot be the last one, so we never need to insert a new empty workspace.

        if allow_to_activate_workspace && activate {
            self.activate_workspace(workspace_idx);
            // Shown scratchpad windows following to this workspace shouldn't take the focus.
            self.active_workspace().activate_window(&id);
        }
    }

    pub fn clean_up_workspaces(&mut self) {
        assert!(self.workspace_switch.is_none());

        let range_start = if self.options.layout.empty_workspace_above_first {
            1
        } else {
            0
        };
        for idx in (range_start..self.workspaces.len() - 1).rev() {
            if self.active_workspace_idx == idx {
                continue;
            }

            if !self.workspaces[idx].has_windows_or_name() {
                self.workspaces.remove(idx);
                if self.active_workspace_idx > idx {
                    self.active_workspace_idx -= 1;
                }
            }
        }

        // Special case handling when empty_workspace_above_first is set and all workspaces
        // are empty.
        if self.options.layout.empty_workspace_above_first && self.workspaces.len() == 2 {
            assert!(!self.workspaces[0].has_windows_or_name());
            assert!(!self.workspaces[1].has_windows_or_name());
            self.workspaces.remove(1);
            self.active_workspace_idx = 0;
        }
    }

    /// Cleans up the workspace at `idx` after removing a window from it.
    pub(super) fn clean_up_workspace_after_removal(&mut self, idx: usize) {
        // Clean up empty workspaces that are not active and not last.
        if !self.workspaces[idx].has_windows_or_name()
            && idx != self.active_workspace_idx
            && idx != self.workspaces.len() - 1
            && self.workspace_switch.is_none()
        {
            self.workspaces.remove(idx);

            if idx < self.active_workspace_idx {
                self.active_workspace_idx -= 1;
            }
        }

        // Special case handling when empty_workspace_above_first is set and all workspaces are
        // empty.
        if self.options.layout.empty_workspace_above_first
            && self.workspaces.len() == 2
            && self.workspace_switch.is_none()
        {
            assert!(!self.workspaces[0].has_windows_or_name());
            assert!(!self.workspaces[1].has_windows_or_name());
            self.workspaces.remove(1);
            self.active_workspace_idx = 0;
        }
    }

    pub fn unname_workspace(&mut self, id: WorkspaceId) -> bool {
        let Some(idx) = self.idx_of_ws(id) else {
            return false;
        };
        let ws = &mut self.workspaces[idx];

        ws.unname();

        if self.workspace_switch.is_none() {
            self.clean_up_workspaces();
        }

        true
    }

    pub fn remove_workspace_by_idx(&mut self, mut idx: usize) -> Workspace<W> {
        if idx == self.workspaces.len() - 1 {
            self.add_workspace_bottom();
        }
        if self.options.layout.empty_workspace_above_first && idx == 0 {
            self.add_workspace_top();
            idx += 1;
        }

        let mut ws = self.workspaces.remove(idx);
        ws.set_output(None);

        // For monitor current workspace removal, we focus previous rather than next (<= rather
        // than <). This is different from columns and tiles, but it lets move-workspace-to-monitor
        // back and forth to preserve position.
        if idx <= self.active_workspace_idx && self.active_workspace_idx > 0 {
            self.active_workspace_idx -= 1;
        }

        self.workspace_switch = None;
        self.clean_up_workspaces();

        ws
    }

    pub fn insert_workspace(&mut self, mut ws: Workspace<W>, mut idx: usize, activate: bool) {
        ws.set_output(Some(self.output.clone()));
        ws.update_config(self.options.clone());

        // Don't insert past the last empty workspace.
        if idx == self.workspaces.len() {
            idx -= 1;
        }
        if idx == 0 && self.options.layout.empty_workspace_above_first {
            // Insert a new empty workspace on top to prepare for insertion of new workspace.
            self.add_workspace_top();
            idx += 1;
        }

        self.workspaces.insert(idx, ws);

        if idx <= self.active_workspace_idx {
            self.active_workspace_idx += 1;
        }

        if activate {
            self.workspace_switch = None;
            self.activate_workspace(idx);
        }

        self.workspace_switch = None;
        self.clean_up_workspaces();

        // The workspace could've been inserted without activating.
        self.scratchpad_follow();
    }

    pub fn append_workspaces(&mut self, mut workspaces: Vec<Workspace<W>>) {
        if workspaces.is_empty() {
            return;
        }

        for ws in &mut workspaces {
            ws.set_output(Some(self.output.clone()));
            ws.update_config(self.options.clone());
        }

        let empty_was_focused = self.active_workspace_idx == self.workspaces.len() - 1;

        // Push the workspaces from the removed monitor in the end, right before the
        // last, empty, workspace.
        let empty = self.workspaces.remove(self.workspaces.len() - 1);
        self.workspaces.extend(workspaces);
        self.workspaces.push(empty);

        // If empty_workspace_above_first is set and the first workspace is now no longer empty,
        // add a new empty workspace on top.
        if self.options.layout.empty_workspace_above_first
            && self.workspaces[0].has_windows_or_name()
        {
            self.add_workspace_top();
        }

        // If the empty workspace was focused on the primary monitor, keep it focused.
        if empty_was_focused {
            self.active_workspace_idx = self.workspaces.len() - 1;
        }

        // FIXME: if we're adding workspaces to currently invisible positions
        // (outside the workspace switch), we don't need to cancel it.
        self.workspace_switch = None;
        self.clean_up_workspaces();

        self.scratchpad_follow();
    }

    pub fn move_down_or_to_workspace_down(&mut self) {
        if !self.active_workspace().move_down() {
            self.move_to_workspace_down(ActivateWindow::Smart);
        }
    }

    pub fn move_up_or_to_workspace_up(&mut self) {
        if !self.active_workspace().move_up() {
            self.move_to_workspace_up(ActivateWindow::Smart);
        }
    }

    pub fn focus_window_or_workspace_down(&mut self) {
        if !self.active_workspace().focus_down() {
            self.switch_workspace_down();
        }
    }

    pub fn focus_window_or_workspace_up(&mut self) {
        if !self.active_workspace().focus_up() {
            self.switch_workspace_up();
        }
    }

    pub fn move_to_workspace_up(&mut self, activate: ActivateWindow) {
        let new_idx = self.active_workspace_idx.saturating_sub(1);
        self.move_to_workspace(None, new_idx, activate);
    }

    pub fn move_to_workspace_down(&mut self, activate: ActivateWindow) {
        let new_idx = min(self.active_workspace_idx + 1, self.workspaces.len() - 1);
        self.move_to_workspace(None, new_idx, activate);
    }

    pub fn move_to_workspace(
        &mut self,
        window: Option<&W::Id>,
        idx: usize,
        activate: ActivateWindow,
    ) {
        let source_workspace_idx = if let Some(window) = window {
            self.workspaces
                .iter()
                .position(|ws| ws.has_window(window))
                .unwrap()
        } else {
            self.active_workspace_idx
        };
        let source_id = self.workspaces[source_workspace_idx].id();

        let new_idx = min(idx, self.workspaces.len() - 1);
        if new_idx == source_workspace_idx {
            return;
        }
        let new_id = self.workspaces[new_idx].id();

        let activate = activate.map_smart(|| {
            window.is_none_or(|win| self.active_window().map(|win| win.id()) == Some(win))
        });

        let workspace = &mut self.workspaces[source_workspace_idx];
        let Some(window) = window.or_else(|| workspace.active_window().map(|win| win.id())) else {
            return;
        };
        let window = window.clone();

        let mut old_render_pos = workspace
            .tiles_with_render_positions()
            .find_map(|(tile, offset, _visible)| (tile.window().id() == &window).then_some(offset))
            .unwrap();

        let transaction = Transaction::new();
        let removed = workspace.remove_tile(&window, transaction);

        if !removed.is_floating {
            workspace.fill_view_after_removal();
        }

        // If the view is following the tile, match the animation.
        let config = if activate {
            self.options.animations.workspace_switch.0
        } else {
            self.options.animations.window_movement.0
        };

        self.add_tile(
            removed.tile,
            MonitorAddWindowTarget::Workspace {
                id: new_id,
                column_idx: None,
            },
            if activate {
                ActivateWindow::Yes
            } else {
                ActivateWindow::No
            },
            true,
            removed.width,
            removed.is_full_width,
            removed.is_floating,
            Some(config),
        );

        if self.workspace_switch.is_none() {
            self.clean_up_workspaces();
        }

        let new_idx = self.idx_of_ws(new_id).unwrap();

        // Animate vertical movement between workspaces.
        //
        // Recompute the source idx in case some workspace was removed during clean-up. If the
        // source workspace itself was removed, don't bother animating this since the removal is
        // instant anyway.
        if let Some(source_workspace_idx) = self.idx_of_ws(source_id) {
            old_render_pos.y += self.workspace_distance(source_workspace_idx, new_idx);
        }

        let (tile, new_render_pos) = self.workspaces[new_idx]
            .tiles_with_render_positions_mut(false)
            .find(|(tile, _)| tile.window().id() == &window)
            .unwrap();
        tile.animate_move_from_with_config(old_render_pos - new_render_pos, config);
        tile.set_anim_y_between_workspaces();
    }

    pub fn move_column_to_workspace_up(&mut self, activate: bool) {
        let new_idx = self.active_workspace_idx.saturating_sub(1);
        self.move_column_to_workspace(new_idx, activate);
    }

    pub fn move_column_to_workspace_down(&mut self, activate: bool) {
        let new_idx = min(self.active_workspace_idx + 1, self.workspaces.len() - 1);
        self.move_column_to_workspace(new_idx, activate);
    }

    pub fn move_column_to_workspace(&mut self, idx: usize, activate: bool) {
        let source_workspace_idx = self.active_workspace_idx;

        let new_idx = min(idx, self.workspaces.len() - 1);
        if new_idx == source_workspace_idx {
            return;
        }

        let workspace = &mut self.workspaces[source_workspace_idx];
        if workspace.floating_is_active() {
            let activate = if activate {
                ActivateWindow::Smart
            } else {
                ActivateWindow::No
            };
            self.move_to_workspace(None, idx, activate);
            return;
        }

        let Some(id) = workspace.scrolling().active_column().map(Column::id) else {
            return;
        };
        let mut old_render_pos = workspace
            .scrolling()
            .columns_with_render_positions()
            .find_map(|(col, pos)| (col.id() == id).then_some(pos))
            .unwrap();

        let column = workspace.remove_active_column().unwrap();
        workspace.fill_view_after_removal();

        // If the view is following the column, match the animation.
        let config = if activate {
            self.options.animations.workspace_switch.0
        } else {
            self.options.animations.window_movement.0
        };

        let source_id = self.workspaces[source_workspace_idx].id();
        let new_id = self.workspaces[new_idx].id();
        let fallback_distance = self.workspace_distance(source_workspace_idx, new_idx);
        self.add_column(new_idx, column, activate, Some(config));

        let new_idx = self.idx_of_ws(new_id).unwrap();

        // Animate vertical movement between workspaces. This is after adding the column since
        // that can start a workspace switch that changes how the workspaces are laid out.
        old_render_pos.y += match self.idx_of_ws(source_id) {
            Some(source_workspace_idx) => self.workspace_distance(source_workspace_idx, new_idx),
            None => fallback_distance,
        };

        let (column, new_render_pos) = self.workspaces[new_idx]
            .scrolling_mut()
            .columns_with_render_positions_mut()
            .find(|(col, _pos)| col.id() == id)
            .unwrap();
        column.animate_move_from_with_config(old_render_pos - new_render_pos, config);
        column.set_anim_y_between_workspaces();
    }

    pub fn switch_workspace_up(&mut self) {
        let new_idx = match &self.workspace_switch {
            // During a DnD scroll, select the prev apparent workspace.
            Some(WorkspaceSwitch::Gesture(gesture)) if gesture.dnd_last_event_time.is_some() => {
                let current = gesture.current_idx;
                let new = current.ceil() - 1.;
                new.clamp(0., (self.workspaces.len() - 1) as f64) as usize
            }
            _ => self.active_workspace_idx.saturating_sub(1),
        };

        self.activate_workspace(new_idx);
    }

    pub fn switch_workspace_down(&mut self) {
        let new_idx = match &self.workspace_switch {
            // During a DnD scroll, select the next apparent workspace.
            Some(WorkspaceSwitch::Gesture(gesture)) if gesture.dnd_last_event_time.is_some() => {
                let current = gesture.current_idx;
                let new = current.floor() + 1.;
                new.clamp(0., (self.workspaces.len() - 1) as f64) as usize
            }
            _ => min(self.active_workspace_idx + 1, self.workspaces.len() - 1),
        };

        self.activate_workspace(new_idx);
    }

    fn previous_workspace_idx(&self) -> Option<usize> {
        let id = self.previous_workspace_id?;
        self.idx_of_ws(id)
    }

    pub fn switch_workspace(&mut self, idx: usize) {
        self.activate_workspace(min(idx, self.workspaces.len() - 1));
    }

    pub fn switch_workspace_auto_back_and_forth(&mut self, idx: usize) {
        let idx = min(idx, self.workspaces.len() - 1);

        if idx == self.active_workspace_idx {
            if let Some(prev_idx) = self.previous_workspace_idx() {
                self.switch_workspace(prev_idx);
            }
        } else {
            self.switch_workspace(idx);
        }
    }

    pub fn switch_workspace_previous(&mut self) {
        if let Some(idx) = self.previous_workspace_idx() {
            self.switch_workspace(idx);
        }
    }

    pub fn active_window(&self) -> Option<&W> {
        self.active_workspace_ref().active_window()
    }

    pub fn advance_animations(&mut self) {
        match &mut self.workspace_switch {
            Some(WorkspaceSwitch::Animation(anim)) => {
                if anim.is_done() {
                    self.workspace_switch = None;
                    self.clean_up_workspaces();
                }
            }
            Some(WorkspaceSwitch::Skip(skip)) => {
                if skip.anim.is_done() {
                    self.workspace_switch = None;
                    self.clean_up_workspaces();
                }
            }
            Some(WorkspaceSwitch::Gesture(gesture)) => {
                // Make sure the last event time doesn't go too much out of date (for
                // monitors not under cursor), causing sudden jumps.
                //
                // This happens after any dnd_scroll_gesture_scroll() calls (in
                // Layout::advance_animations()), so it doesn't mess up the time delta there.
                if let Some(last_time) = &mut gesture.dnd_last_event_time {
                    let now = self.clock.now_unadjusted();
                    if *last_time != now {
                        *last_time = now;

                        // If last_time was already == now, then dnd_scroll_gesture_scroll() must've
                        // updated the gesture already. Therefore, when this code runs, the pointer
                        // must be outside the DnD scrolling zone.
                        gesture.dnd_nonzero_start_time = None;
                    }
                }

                if let Some(anim) = &mut gesture.animation {
                    if anim.is_done() {
                        gesture.animation = None;
                    }
                }
            }
            None => (),
        }

        for ws in &mut self.workspaces {
            ws.advance_animations();
        }
    }

    pub(super) fn are_animations_ongoing(&self) -> bool {
        self.workspace_switch
            .as_ref()
            .is_some_and(|s| s.is_animation_ongoing())
            || self.workspaces.iter().any(|ws| ws.are_animations_ongoing())
    }

    pub fn are_transitions_ongoing(&self) -> bool {
        self.workspace_switch.is_some()
            || self
                .workspaces
                .iter()
                .any(|ws| ws.are_transitions_ongoing())
    }

    pub fn update_render_elements(&mut self, is_active: bool) {
        self.update_skip_dimming();

        let mut insert_hint_ws_geo = None;
        let insert_hint_ws_id = self
            .insert_hint
            .as_ref()
            .and_then(|hint| hint.workspace.existing_id());

        let pin_scratchpad = self.scratchpad_pin_geo().is_some();
        if pin_scratchpad {
            // Pinned windows are visible even when their workspace is culled.
            let ws = &mut self.workspaces[self.active_workspace_idx];
            ws.update_render_elements(is_active, RenderLayer::PinnedScratchpad, true);
        }

        for ws in &mut self.workspaces {
            ws.update_render_elements(
                is_active,
                RenderLayer::MovingBetweenWorkspaces,
                pin_scratchpad,
            );
        }

        for (ws, geo) in self.workspaces_with_render_geo_mut(true) {
            ws.update_render_elements(is_active, RenderLayer::Normal, pin_scratchpad);

            if Some(ws.id()) == insert_hint_ws_id {
                insert_hint_ws_geo = Some(geo);
            }
        }

        self.insert_hint_render_loc = None;
        if let Some(hint) = &self.insert_hint {
            match hint.workspace {
                InsertWorkspace::Existing(ws_id) => {
                    if let Some(idx) = self.idx_of_ws(ws_id) {
                        let ws = &self.workspaces[idx];
                        if let Some(mut area) = ws.insert_hint_area(hint.position) {
                            let scale = ws.scale().fractional_scale();
                            let view_size = ws.view_size();

                            // Make sure the hint is at least partially visible.
                            if matches!(hint.position, InsertPosition::NewColumn(_)) {
                                let zoom = self.overview_zoom();
                                let geo = insert_hint_ws_geo.unwrap();
                                let geo = geo.downscale(zoom);

                                area.loc.x = area.loc.x.max(-geo.loc.x - area.size.w / 2.);
                                area.loc.x =
                                    area.loc.x.min(geo.loc.x + geo.size.w - area.size.w / 2.);
                            }

                            // Round to physical pixels.
                            area = area.to_physical_precise_round(scale).to_logical(scale);

                            let view_rect = Rectangle::new(area.loc.upscale(-1.), view_size);
                            self.insert_hint_element.update_render_elements(
                                area.size,
                                view_rect,
                                hint.corner_radius,
                                scale,
                            );
                            self.insert_hint_render_loc = Some(InsertHintRenderLoc {
                                workspace: hint.workspace,
                                location: area.loc,
                            });
                        }
                    } else {
                        error!("insert hint workspace missing from monitor");
                    }
                }
                InsertWorkspace::NewAt(ws_idx) => {
                    let scale = self.scale.fractional_scale();
                    let zoom = self.overview_zoom();
                    let gap = self.workspace_gap(zoom);

                    let hint_gap = round_logical_in_physical(scale, gap * 0.1);
                    let hint_height = gap - hint_gap * 2.;

                    let next_ws_geo = self.workspaces_render_geo().nth(ws_idx).unwrap();
                    let hint_width = round_logical_in_physical(scale, next_ws_geo.size.w * 0.75);
                    let hint_x =
                        round_logical_in_physical(scale, (next_ws_geo.size.w - hint_width) / 2.);

                    let hint_loc_diff = Point::from((-hint_x, hint_height + hint_gap));
                    let hint_loc = next_ws_geo.loc - hint_loc_diff;
                    let hint_size = Size::from((hint_width, hint_height));

                    // Sometimes the hint ends up 1 px wider than necessary and/or 1 px
                    // narrower than necessary. The values here seem correct. Might have to do with
                    // how zooming out currently doesn't round to output scale properly.

                    // Compute view rect as if we're above the next workspace (rather than below
                    // the previous one).
                    let view_rect = Rectangle::new(hint_loc_diff, next_ws_geo.size);

                    self.insert_hint_element.update_render_elements(
                        hint_size,
                        view_rect,
                        CornerRadius::default(),
                        scale,
                    );
                    self.insert_hint_render_loc = Some(InsertHintRenderLoc {
                        workspace: hint.workspace,
                        location: hint_loc,
                    });
                }
            }
        }
    }

    pub fn update_config(&mut self, base_options: Rc<Options>) {
        let options =
            Rc::new(Options::clone(&base_options).with_merged_layout(self.layout_config.as_ref()));

        if self.options.layout.empty_workspace_above_first
            != options.layout.empty_workspace_above_first
            && self.workspaces.len() > 1
        {
            if options.layout.empty_workspace_above_first {
                self.add_workspace_top();
            } else if self.workspace_switch.is_none() && self.active_workspace_idx != 0 {
                self.workspaces.remove(0);
                self.active_workspace_idx = self.active_workspace_idx.saturating_sub(1);
            }
        }

        for ws in &mut self.workspaces {
            ws.update_config(options.clone());
        }

        self.insert_hint_element
            .update_config(options.layout.insert_hint);

        self.base_options = base_options;
        self.options = options;
    }

    pub fn update_layout_config(&mut self, layout_config: Option<niri_config::LayoutPart>) -> bool {
        if self.layout_config == layout_config {
            return false;
        }

        self.layout_config = layout_config;
        self.update_config(self.base_options.clone());

        true
    }

    pub fn update_shaders(&mut self) {
        for ws in &mut self.workspaces {
            ws.update_shaders();
        }

        self.insert_hint_element.update_shaders();
    }

    pub fn update_output_size(&mut self) {
        self.scale = self.output.current_scale();
        self.view_size = output_size(&self.output);
        self.working_area = compute_working_area(&self.output);

        for ws in &mut self.workspaces {
            ws.update_output_size();
        }
    }

    pub fn move_workspace_down(&mut self) {
        let mut new_idx = min(self.active_workspace_idx + 1, self.workspaces.len() - 1);
        if new_idx == self.active_workspace_idx {
            return;
        }

        self.workspaces.swap(self.active_workspace_idx, new_idx);

        if new_idx == self.workspaces.len() - 1 {
            // Insert a new empty workspace.
            self.add_workspace_bottom();
        }

        if self.options.layout.empty_workspace_above_first && self.active_workspace_idx == 0 {
            self.add_workspace_top();
            new_idx += 1;
        }

        let previous_workspace_id = self.previous_workspace_id;
        self.activate_workspace(new_idx);
        self.workspace_switch = None;
        self.previous_workspace_id = previous_workspace_id;

        self.clean_up_workspaces();
    }

    pub fn move_workspace_up(&mut self) {
        let mut new_idx = self.active_workspace_idx.saturating_sub(1);
        if new_idx == self.active_workspace_idx {
            return;
        }

        self.workspaces.swap(self.active_workspace_idx, new_idx);

        if self.active_workspace_idx == self.workspaces.len() - 1 {
            // Insert a new empty workspace.
            self.add_workspace_bottom();
        }

        if self.options.layout.empty_workspace_above_first && new_idx == 0 {
            self.add_workspace_top();
            new_idx += 1;
        }

        let previous_workspace_id = self.previous_workspace_id;
        self.activate_workspace(new_idx);
        self.workspace_switch = None;
        self.previous_workspace_id = previous_workspace_id;

        self.clean_up_workspaces();
    }

    pub fn move_workspace_to_idx(&mut self, old_idx: usize, new_idx: usize) {
        if self.workspaces.len() <= old_idx {
            return;
        }

        let new_idx = new_idx.clamp(0, self.workspaces.len() - 1);
        if old_idx == new_idx {
            return;
        }

        let active_id = self.workspaces[self.active_workspace_idx].id();

        let ws = self.workspaces.remove(old_idx);
        self.workspaces.insert(new_idx, ws);

        if new_idx > old_idx {
            if new_idx == self.workspaces.len() - 1 {
                // Insert a new empty workspace.
                self.add_workspace_bottom();
            }

            if self.options.layout.empty_workspace_above_first && old_idx == 0 {
                self.add_workspace_top();
            }
        } else {
            if old_idx == self.workspaces.len() - 1 {
                // Insert a new empty workspace.
                self.add_workspace_bottom();
            }

            if self.options.layout.empty_workspace_above_first && new_idx == 0 {
                self.add_workspace_top();
            }
        }

        // Keep the same workspace active. Adding the empty workspaces above may have already
        // changed active_workspace_idx, so look it up rather than adjusting it.
        self.active_workspace_idx = self.idx_of_ws(active_id).unwrap();

        self.workspace_switch = None;

        self.clean_up_workspaces();

        // The active workspace is not supposed to change here, but follow just in case.
        self.scratchpad_follow();
    }

    /// Returns the geometry of the active window relative to and clamped to the output.
    ///
    /// During animations, assumes the final view position.
    pub fn active_window_visual_rectangle(&self) -> Option<Rectangle<f64, Logical>> {
        if self.overview_open {
            return None;
        }

        self.active_workspace_ref().active_window_visual_rectangle()
    }

    fn workspace_size(&self, zoom: f64) -> Size<f64, Logical> {
        let ws_size = self.view_size.upscale(zoom);
        let scale = self.scale.fractional_scale();
        ws_size.to_physical_precise_ceil(scale).to_logical(scale)
    }

    fn workspace_gap(&self, zoom: f64) -> f64 {
        let scale = self.scale.fractional_scale();
        let gap = self.view_size.h * 0.1 * zoom;
        round_logical_in_physical_max1(scale, gap)
    }

    fn workspace_size_with_gap(&self, zoom: f64) -> Size<f64, Logical> {
        let gap = self.workspace_gap(zoom);
        self.workspace_size(zoom) + Size::from((0., gap))
    }

    /// Offset that centers a workspace of the given size horizontally and vertically.
    fn workspace_static_offset(&self, ws_size: Size<f64, Logical>) -> Point<f64, Logical> {
        let scale = self.scale.fractional_scale();
        let offset = (self.view_size.to_point() - ws_size.to_point()).downscale(2.);
        offset.to_physical_precise_round(scale).to_logical(scale)
    }

    pub fn overview_zoom(&self) -> f64 {
        let progress = self.overview_progress.as_ref().map(|p| p.value());
        compute_overview_zoom(&self.options, progress)
    }

    pub(super) fn set_overview_progress(&mut self, progress: Option<&super::OverviewProgress>) {
        // The overview shows all workspaces in order, so it can't show a skipping switch.
        if progress.is_some() {
            self.collapse_workspace_switch_skip();
        }

        let prev_render_idx = self.workspace_render_idx();
        let prev_pin_geo = self.scratchpad_pin_geo();
        let prev_active_geo = self.active_workspace_render_geo();
        self.overview_progress = progress.map(OverviewProgress::from);
        let new_render_idx = self.workspace_render_idx();

        // If the view jumped (can happen when going from corrected to uncorrected render_idx, for
        // example when toggling the overview in the middle of an overview animation), then restart
        // the workspace switch to avoid jumps.
        if prev_render_idx != new_render_idx {
            if let Some(WorkspaceSwitch::Animation(anim)) = &mut self.workspace_switch {
                // FIXME: maintain velocity.
                *anim = anim.restarted(prev_render_idx, anim.to(), 0.);
            }
        }

        // If the shown scratchpad windows got pinned or unpinned in the middle of a workspace
        // switch, animate them from where they were. The overview zoom is 1 at this point.
        let from = match (prev_pin_geo, self.scratchpad_pin_geo()) {
            (Some(pin_geo), None) => pin_geo.loc - self.active_workspace_render_geo().loc,
            (None, Some(pin_geo)) => prev_active_geo.loc - pin_geo.loc,
            _ => return,
        };
        if from == Point::default() {
            return;
        }

        let config = self.options.animations.workspace_switch.0;
        let ws = &mut self.workspaces[self.active_workspace_idx];
        for (tile, _) in ws.tiles_with_render_positions_mut(false) {
            if tile.is_pinned() {
                tile.animate_move_from_with_config(from, config);
                tile.set_anim_y_between_workspaces();
            }
        }
    }

    fn active_workspace_render_geo(&self) -> Rectangle<f64, Logical> {
        self.workspaces_render_geo()
            .nth(self.active_workspace_idx)
            .unwrap()
    }

    #[cfg(test)]
    pub(super) fn overview_progress_value(&self) -> Option<f64> {
        self.overview_progress.as_ref().map(|p| p.value())
    }

    pub fn workspace_render_idx(&self) -> f64 {
        // If workspace switch and overview progress are matching animations, then compute a
        // correction term to make the movement appear monotonic.
        if let (
            Some(WorkspaceSwitch::Animation(switch_anim)),
            Some(OverviewProgress::Animation(progress_anim)),
        ) = (&self.workspace_switch, &self.overview_progress)
        {
            if switch_anim.start_time() == progress_anim.start_time()
                && (switch_anim.duration().as_secs_f64() - progress_anim.duration().as_secs_f64())
                    .abs()
                    <= 0.001
            {
                #[rustfmt::skip]
                // How this was derived:
                //
                // - Assume we're animating a zoom + switch. Consider switch "from" and "to".
                //   These are render_idx values, so first workspace to second would have switch
                //   from = 0. and to = 1. regardless of the zoom level.
                //
                // - At the start, the point at "from" is at Y = 0. We're moving the point at "to"
                //   to Y = 0. We want this to be a monotonic motion in apparent coordinates (after
                //   zoom).
                //
                // - Height at the start:
                //   from_height = (size.h + gap) * from_zoom.
                //
                // - Current height:
                //   current_height = (size.h + gap) * zoom.
                //
                // - We're moving the "to" point to Y = 0:
                //   to_y = 0.
                //
                // - The initial position of the point we're moving:
                //   from_y = (to - from) * from_height.
                //
                // - We want this point to travel monotonically in apparent coordinates:
                //   current_y = from_y + (to_y - from_y) * progress,
                //   where progress is from 0 to 1, equals to the animation progress (switch and
                //   zoom are the same since they are synchronized).
                //
                // - Derive the Y of the first workspace from this:
                //   first_y = current_y - to * current_height.
                //
                // Now, let's substitute and rearrange the terms.
                //
                // - current_y = from_y + (0 - (to - from) * from_height) * progress
                // - progress = (switch_anim.value() - from) / (to - from)
                // - current_y = from_y - (to - from) * from_height * (switch_anim.value() - from) / (to - from)
                // - current_y = from_y - from_height * (switch_anim.value() - from)
                // - first_y = from_y - from_height * (switch_anim.value() - from) - to * current_height
                // - first_y = (to - from) * from_height - from_height * (switch_anim.value() - from) - to * current_height
                // - first_y = to * from_height - switch_anim.value() * from_height - to * current_height
                // - first_y = -switch_anim.value() * from_height + to * (from_height - current_height)
                let from = progress_anim.from();
                let from_zoom = compute_overview_zoom(&self.options, Some(from));
                let from_ws_height_with_gap = self.workspace_size_with_gap(from_zoom).h;

                let zoom = self.overview_zoom();
                let ws_height_with_gap = self.workspace_size_with_gap(zoom).h;

                let first_ws_y = -switch_anim.value() * from_ws_height_with_gap
                    + switch_anim.to() * (from_ws_height_with_gap - ws_height_with_gap);

                return -first_ws_y / ws_height_with_gap;
            }
        };

        match &self.workspace_switch {
            Some(WorkspaceSwitch::Skip(skip)) => self.skip_render_idx(skip),
            Some(switch) => switch.current_idx().unwrap(),
            None => self.active_workspace_idx as f64,
        }
    }

    /// Returns the skipping workspace switch, if one is in progress.
    fn workspace_switch_skip(&self) -> Option<&WorkspaceSwitchSkip> {
        match &self.workspace_switch {
            Some(WorkspaceSwitch::Skip(skip)) => Some(skip),
            _ => None,
        }
    }

    /// Vertical distance from the workspace at `new_idx` to the one at `source_idx`, as laid out.
    ///
    /// Used to move things between workspaces. Normally that's the distance between the
    /// workspaces in the strip, but a skipping workspace switch shows them differently.
    fn workspace_distance(&self, source_idx: usize, new_idx: usize) -> f64 {
        let ws_height_with_gap = self.workspace_size_with_gap(1.).h;

        if let Some(skip) = self.workspace_switch_skip() {
            let positions = self.skip_positions(skip);
            let find = |idx| positions.iter().find(|pos| pos.idx == idx).map(|pos| pos.y);
            if let (Some(source_y), Some(new_y)) = (find(source_idx), find(new_idx)) {
                return (source_y - new_y) * ws_height_with_gap;
            }
        }

        ws_height_with_gap * (source_idx as f64 - new_idx as f64)
    }

    /// Current positions of the items of a skipping workspace switch.
    ///
    /// Items whose workspace is gone are left out.
    fn skip_positions(&self, skip: &WorkspaceSwitchSkip) -> Vec<SkipPos> {
        let progress = skip.anim.value();
        skip.items
            .iter()
            .filter_map(|item| {
                Some(SkipPos {
                    idx: self.idx_of_ws(item.id)?,
                    y: item.y.at(progress),
                    top: item.top.at(progress),
                    bottom: item.bottom.at(progress),
                })
            })
            .collect()
    }

    /// Workspace render index that puts the most visible workspace of a skipping switch in place.
    fn skip_render_idx(&self, skip: &WorkspaceSwitchSkip) -> f64 {
        self.skip_positions(skip)
            .into_iter()
            .max_by(|a, b| a.on_screen().total_cmp(&b.on_screen()))
            .map_or(skip.target_idx, |pos| pos.idx as f64 - pos.y)
    }

    /// Replaces a skipping workspace switch with a regular one, if there is one in progress.
    ///
    /// The most visible workspace stays where it is. The others may jump.
    fn collapse_workspace_switch_skip(&mut self) {
        let Some(WorkspaceSwitch::Skip(skip)) = &self.workspace_switch else {
            return;
        };

        let from = self.skip_render_idx(skip);
        let config = self.options.animations.workspace_switch.0;
        self.workspace_switch = Some(WorkspaceSwitch::Animation(Animation::new(
            self.clock.clone(),
            from,
            self.active_workspace_idx as f64,
            0.,
            config,
        )));
    }

    /// Starts a skipping workspace switch to `target_idx`, if that is warranted.
    ///
    /// Switching between workspaces that aren't next to each other skips the workspaces in
    /// between. This should be called before changing the active workspace.
    fn start_workspace_switch_skip(
        &self,
        target_idx: usize,
        config: niri_config::Animation,
    ) -> Option<WorkspaceSwitchSkip> {
        // The overview needs the real layout of all workspaces.
        if self.overview_progress.is_some() {
            return None;
        }

        // Workspaces that are currently on screen, with their positions.
        let mut on_screen: Vec<SkipPos> =
            if let Some(WorkspaceSwitch::Skip(skip)) = &self.workspace_switch {
                // Continue from wherever the current skipping switch is.
                self.skip_positions(skip)
            } else {
                let render_idx = self.workspace_render_idx();
                let first = (render_idx - 1.).floor().max(0.) as usize;
                let last = ((render_idx + 1.).ceil() as usize).min(self.workspaces.len() - 1);
                (first..=last)
                    .map(|idx| SkipPos {
                        idx,
                        y: idx as f64 - render_idx,
                        top: f64::NEG_INFINITY,
                        bottom: f64::INFINITY,
                    })
                    .collect()
            };
        on_screen.retain(|pos| pos.on_screen() > 0.);

        // This shouldn't happen, but with nothing on screen there's nothing to place the target
        // next to, so fall back to a regular switch.
        if on_screen.is_empty() {
            return None;
        }

        if !matches!(self.workspace_switch, Some(WorkspaceSwitch::Skip(_))) {
            // If the target is right next to what's on screen, or to where we're already going,
            // then it's a regular scroll. This makes holding down a key to go through the
            // workspaces one by one look the same as always, even if the view lags behind.
            let lowest = on_screen.iter().map(|pos| pos.idx).min().unwrap();
            let highest = on_screen.iter().map(|pos| pos.idx).max().unwrap();
            let near_view = lowest.saturating_sub(1)..=highest + 1;
            let near_target =
                self.active_workspace_idx.saturating_sub(1)..=self.active_workspace_idx + 1;
            if near_view.contains(&target_idx) || near_target.contains(&target_idx) {
                return None;
            }
        }

        // Find where the target goes: its position, and the part of the screen it takes up now.
        // Everything above that part ends up above the screen, everything below it below.
        let target = on_screen.iter().position(|pos| pos.idx == target_idx);
        let (target_y, boundary_top, boundary_bottom) = match target {
            // Already visible, for example when going back.
            Some(i) => {
                let pos = on_screen.remove(i);
                let (top, bottom) = pos.visible();
                (pos.y, top, bottom)
            }
            None => {
                // The target goes between its neighbors in the real order, right next to the more
                // visible one. There's no room for it there, so it comes in from under the less
                // visible one, which slides out of view at the same time.
                let above = on_screen
                    .iter()
                    .filter(|pos| pos.idx < target_idx)
                    .max_by_key(|pos| pos.idx);
                let below = on_screen
                    .iter()
                    .filter(|pos| pos.idx > target_idx)
                    .min_by_key(|pos| pos.idx);
                let next_to_below = match (above, below) {
                    (Some(above), Some(below)) => below.on_screen() > above.on_screen(),
                    (above, _) => above.is_none(),
                };

                if next_to_below {
                    let edge = below.unwrap().visible().0;
                    (edge - 1., edge, edge)
                } else {
                    let edge = above.unwrap().visible().1;
                    (edge, edge, edge)
                }
            }
        };

        // How much the parts above and below the target move to leave the screen to the target.
        let shift_up = -boundary_top;
        let shift_down = 1. - boundary_bottom;

        let middle = (boundary_top + boundary_bottom) / 2.;
        let mut items: Vec<_> = on_screen
            .iter()
            .map(|pos| {
                let (top, bottom) = pos.visible();
                let shift = if (top + bottom) / 2. < middle {
                    shift_up
                } else {
                    shift_down
                };
                SkipItem {
                    id: self.workspaces[pos.idx].id(),
                    y: SkipLine::new(pos.y, shift),
                    top: SkipLine::new(pos.top, shift),
                    bottom: SkipLine::new(pos.bottom, shift),
                }
            })
            .collect();

        // The target moves into place, and its visible part follows the parts around it. It never
        // shows more than the target itself: its top edge goes down from at most boundary_top at
        // the same rate as boundary_top does, and the same for the bottom edge.
        items.push(SkipItem {
            id: self.workspaces[target_idx].id(),
            y: SkipLine::new(target_y, -target_y),
            top: SkipLine::new(boundary_top, shift_up),
            bottom: SkipLine::new(boundary_bottom, shift_down),
        });

        Some(WorkspaceSwitchSkip {
            anim: Animation::new(self.clock.clone(), 0., 1., 0., config),
            items,
            target_idx: target_idx as f64,
        })
    }

    /// Workspace indices and how much to darken them, during a skipping workspace switch.
    pub(super) fn skip_dimming(&self) -> Vec<(usize, f32)> {
        let Some(skip) = self.workspace_switch_skip() else {
            return Vec::new();
        };

        self.skip_positions(skip)
            .into_iter()
            .map(|pos| (pos.idx, WorkspaceSwitchSkip::dim_alpha(pos.y)))
            .filter(|(_, alpha)| *alpha > 0.)
            .collect()
    }

    /// Part of the workspace at `idx` with render geometry `geo` that isn't covered by others.
    ///
    /// Returns `None` when nothing covers it, `Some` with an empty rectangle when it's fully
    /// covered.
    pub fn workspace_render_clip(
        &self,
        idx: usize,
        geo: Rectangle<f64, Logical>,
    ) -> Option<Rectangle<f64, Logical>> {
        let skip = self.workspace_switch_skip()?;
        let pos = self
            .skip_positions(skip)
            .into_iter()
            .find(|pos| pos.idx == idx)?;
        if pos.top <= pos.y && pos.y + 1. <= pos.bottom {
            return None;
        }

        let scale = self.scale.fractional_scale();
        let ws_height_with_gap = self.workspace_size_with_gap(1.).h;
        let offset = self.workspace_static_offset(self.workspace_size(1.)).y;
        let to_output = |y: f64| round_logical_in_physical(scale, y * ws_height_with_gap) + offset;

        let (top, bottom) = pos.visible();
        let top = to_output(top).max(geo.loc.y);
        let bottom = to_output(bottom).min(geo.loc.y + geo.size.h);
        let height = (bottom - top).max(0.);
        Some(Rectangle::new(
            Point::from((geo.loc.x, top)),
            Size::from((geo.size.w, height)),
        ))
    }

    /// Updates the buffers for darkening workspaces during a skipping workspace switch.
    fn update_skip_dimming(&mut self) {
        self.skip_dim_geos.clear();

        let dimming = self.skip_dimming();
        if dimming.is_empty() {
            return;
        }

        let geos: Vec<_> = self.workspaces_render_geo().collect();
        let clips = self.workspace_render_clips();
        for (idx, alpha) in dimming {
            // Only darken the part of the workspace that isn't covered.
            let geo = clips
                .as_ref()
                .and_then(|clips| clips[idx])
                .unwrap_or(geos[idx]);
            if geo.size.h <= 0. {
                continue;
            }

            let i = self.skip_dim_geos.len();
            if self.skip_dim_buffers.len() == i {
                self.skip_dim_buffers.push(SolidColorBuffer::default());
            }
            self.skip_dim_buffers[i].update(geo.size, Color32F::BLACK * alpha);
            self.skip_dim_geos.push(geo);
        }
    }

    pub fn workspaces_render_geo(&self) -> impl Iterator<Item = Rectangle<f64, Logical>> {
        let scale = self.scale.fractional_scale();
        let zoom = self.overview_zoom();

        let ws_size = self.workspace_size(zoom);
        let gap = self.workspace_gap(zoom);
        let ws_height_with_gap = ws_size.h + gap;

        let static_offset = self.workspace_static_offset(ws_size);

        let first_ws_y = -self.workspace_render_idx() * ws_height_with_gap;
        let first_ws_y = round_logical_in_physical(scale, first_ws_y);

        let skip_ys = self
            .workspace_switch_skip()
            .map(|skip| self.skip_positions(skip));

        // Return position for one-past-last workspace too.
        (0..=self.workspaces.len()).map(move |idx| {
            let y = match &skip_ys {
                Some(ys) => {
                    // Workspaces that aren't a part of the switch are far away, out of view.
                    let y = ys
                        .iter()
                        .find(|pos| pos.idx == idx)
                        .map_or(SKIP_OUT_OF_VIEW_Y, |pos| pos.y * ws_height_with_gap);
                    round_logical_in_physical(scale, y)
                }
                None => first_ws_y + idx as f64 * ws_height_with_gap,
            };
            let loc = Point::from((0., y)) + static_offset;

            // Even though all components that go into loc are rounded to physical pixels, the
            // floating point addition may lose precision. This can result for example in the
            // current workspace having y = 0.0000000000002 and thus missing pointer hits at the
            // monitor edge with y = 0. So, post-round the location too.
            let loc = loc.to_physical_precise_round(scale).to_logical(scale);

            Rectangle::new(loc, ws_size)
        })
    }

    /// Geometry to render and hit-test the shown scratchpad windows of the active workspace with,
    /// when they are pinned to the screen.
    ///
    /// They are pinned during a workspace switch outside the overview, at the place where the
    /// active workspace is at rest.
    pub fn scratchpad_pin_geo(&self) -> Option<Rectangle<f64, Logical>> {
        if self.overview_progress.is_some() || self.workspace_switch.is_none() {
            return None;
        }

        let ws_size = self.workspace_size(1.);
        Some(Rectangle::new(
            self.workspace_static_offset(ws_size),
            ws_size,
        ))
    }

    /// Parts of the workspaces that aren't covered by others, see
    /// [`Monitor::workspace_render_clip()`].
    ///
    /// Returns `None` when there's no skipping workspace switch, so nothing can be covered.
    fn workspace_render_clips(&self) -> Option<Vec<Option<Rectangle<f64, Logical>>>> {
        self.workspace_switch_skip()?;
        Some(
            zip(0..self.workspaces.len(), self.workspaces_render_geo())
                .map(|(idx, geo)| self.workspace_render_clip(idx, geo))
                .collect(),
        )
    }

    pub fn workspaces_with_render_geo_cull(
        &self,
        cull: bool,
    ) -> impl Iterator<Item = (&Workspace<W>, Rectangle<f64, Logical>)> {
        self.workspaces_with_render_geo_and_clip_cull(cull)
            .map(|(ws, geo, _clip)| (ws, geo))
    }

    pub fn workspaces_with_render_geo(
        &self,
    ) -> impl Iterator<Item = (&Workspace<W>, Rectangle<f64, Logical>)> {
        self.workspaces_with_render_geo_cull(true)
    }

    /// Like [`Monitor::workspaces_with_render_geo_cull()`], but also returns the part of each
    /// workspace that isn't covered by others, if it's cut off.
    pub fn workspaces_with_render_geo_and_clip_cull(
        &self,
        cull: bool,
    ) -> impl Iterator<Item = WorkspaceGeoAndClip<'_, W>> {
        let output_geo = Rectangle::from_size(self.view_size);
        let clips = self.workspace_render_clips();

        let geo = self.workspaces_render_geo();
        zip(self.workspaces.iter().enumerate(), geo)
            .map(move |((idx, ws), geo)| (ws, geo, clips.as_ref().and_then(|clips| clips[idx])))
            // Cull out workspaces outside the output.
            .filter(move |(_ws, geo, clip)| !cull || is_on_output(*geo, *clip, output_geo))
    }

    pub fn workspaces_with_render_geo_and_clip(
        &self,
    ) -> impl Iterator<Item = WorkspaceGeoAndClip<'_, W>> {
        self.workspaces_with_render_geo_and_clip_cull(true)
    }

    pub fn workspaces_with_render_geo_idx(
        &self,
    ) -> impl Iterator<Item = ((usize, &Workspace<W>), Rectangle<f64, Logical>)> {
        let output_geo = Rectangle::from_size(self.view_size);
        let clips = self.workspace_render_clips();

        let geo = self.workspaces_render_geo();
        zip(self.workspaces.iter().enumerate(), geo)
            // Cull out workspaces outside the output.
            .filter(move |((idx, _ws), geo)| {
                let clip = clips.as_ref().and_then(|clips| clips[*idx]);
                is_on_output(*geo, clip, output_geo)
            })
    }

    pub fn workspaces_with_render_geo_mut(
        &mut self,
        cull: bool,
    ) -> impl Iterator<Item = (&mut Workspace<W>, Rectangle<f64, Logical>)> {
        let output_geo = Rectangle::from_size(self.view_size);
        let clips = self.workspace_render_clips();

        let geo: Vec<_> = self.workspaces_render_geo().collect();
        zip(self.workspaces.iter_mut().enumerate(), geo)
            // Cull out workspaces outside the output.
            .filter(move |((idx, _ws), geo)| {
                let clip = clips.as_ref().and_then(|clips| clips[*idx]);
                !cull || is_on_output(*geo, clip, output_geo)
            })
            .map(|((_idx, ws), geo)| (ws, geo))
    }

    pub fn workspace_under(
        &self,
        pos_within_output: Point<f64, Logical>,
    ) -> Option<(&Workspace<W>, Rectangle<f64, Logical>)> {
        let (ws, geo) =
            self.workspaces_with_render_geo_and_clip()
                .find_map(|(ws, geo, clip)| {
                    // Extend width to entire output.
                    let visible = clip.unwrap_or(geo);
                    let loc = Point::from((0., visible.loc.y));
                    let size = Size::from((self.view_size.w, visible.size.h));
                    let bounds = Rectangle::new(loc, size);

                    bounds.contains(pos_within_output).then_some((ws, geo))
                })?;
        Some((ws, geo))
    }

    pub fn workspace_under_narrow(
        &self,
        pos_within_output: Point<f64, Logical>,
    ) -> Option<&Workspace<W>> {
        self.workspaces_with_render_geo_and_clip()
            .find_map(|(ws, geo, clip)| {
                let visible = clip.unwrap_or(geo);
                visible.contains(pos_within_output).then_some(ws)
            })
    }

    pub fn window_under(&self, pos_within_output: Point<f64, Logical>) -> Option<(&W, HitType)> {
        // Pinned scratchpad windows are on top of everything else.
        let pin_geo = self.scratchpad_pin_geo();
        if let Some(geo) = pin_geo {
            let ws = &self.workspaces[self.active_workspace_idx];
            let pos_within_workspace = pos_within_output - geo.loc;
            if let Some((win, hit)) =
                ws.scratchpad_tiles_with_render_positions()
                    .find_map(|(tile, tile_pos)| {
                        HitType::hit_tile(tile, tile_pos, pos_within_workspace)
                    })
            {
                return Some((win, hit.offset_win_pos(geo.loc)));
            }
        }

        let (ws, geo) = self.workspace_under(pos_within_output)?;

        if self.overview_progress.is_some() {
            let zoom = self.overview_zoom();
            let pos_within_workspace = (pos_within_output - geo.loc).downscale(zoom);
            let (win, hit) = ws.window_under(pos_within_workspace, false)?;
            // During the overview animation, we cannot do input hits because we cannot really
            // represent scaled windows properly.
            Some((win, hit.to_activate()))
        } else {
            let (win, hit) = ws.window_under(pos_within_output - geo.loc, pin_geo.is_some())?;
            Some((win, hit.offset_win_pos(geo.loc)))
        }
    }

    pub fn resize_edges_under(&self, pos_within_output: Point<f64, Logical>) -> Option<ResizeEdge> {
        if self.overview_progress.is_some() {
            return None;
        }

        // Pinned scratchpad windows are on top of everything else.
        let pin_geo = self.scratchpad_pin_geo();
        if let Some(geo) = pin_geo {
            let ws = &self.workspaces[self.active_workspace_idx];
            let pos_within_workspace = pos_within_output - geo.loc;
            if let Some(edges) =
                ws.scratchpad_tiles_with_render_positions()
                    .find_map(|(tile, tile_pos)| {
                        resize_edges_for_tile(tile, pos_within_workspace - tile_pos)
                    })
            {
                return Some(edges);
            }
        }

        let (ws, geo) = self.workspace_under(pos_within_output)?;
        ws.resize_edges_under(pos_within_output - geo.loc, pin_geo.is_some())
    }

    pub(super) fn insert_position(
        &self,
        pos_within_output: Point<f64, Logical>,
    ) -> (InsertWorkspace, Rectangle<f64, Logical>) {
        let mut iter = self.workspaces_with_render_geo_idx();

        let dummy = Rectangle::default();

        // Monitors always have at least one workspace.
        let ((idx, ws), geo) = iter.next().unwrap();

        // Check if above first.
        if pos_within_output.y < geo.loc.y {
            return (InsertWorkspace::NewAt(idx), dummy);
        }

        let contains = move |geo: Rectangle<f64, Logical>| {
            geo.loc.y <= pos_within_output.y && pos_within_output.y < geo.loc.y + geo.size.h
        };

        // Check first.
        if contains(geo) {
            return (InsertWorkspace::Existing(ws.id()), geo);
        }

        let mut last_geo = geo;
        let mut last_idx = idx;
        for ((idx, ws), geo) in iter {
            // Check gap above.
            let gap_loc = Point::from((last_geo.loc.x, last_geo.loc.y + last_geo.size.h));
            let gap_size = Size::from((geo.size.w, geo.loc.y - gap_loc.y));
            let gap_geo = Rectangle::new(gap_loc, gap_size);
            if contains(gap_geo) {
                return (InsertWorkspace::NewAt(idx), dummy);
            }

            // Check workspace itself.
            if contains(geo) {
                return (InsertWorkspace::Existing(ws.id()), geo);
            }

            last_geo = geo;
            last_idx = idx;
        }

        // Anything below.
        (InsertWorkspace::NewAt(last_idx + 1), dummy)
    }

    pub fn render_above_top_layer(&self) -> bool {
        // Render above the top layer only if the view is stationary.
        if self.workspace_switch.is_some() || self.overview_progress.is_some() {
            return false;
        }

        let ws = &self.workspaces[self.active_workspace_idx];
        ws.render_above_top_layer()
    }

    pub fn render_insert_hint_between_workspaces<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        push: &mut dyn FnMut(MonitorRenderElement<R>),
    ) {
        if self.options.layout.insert_hint.off {
            return;
        }
        let Some(render_loc) = self.insert_hint_render_loc else {
            return;
        };
        let InsertWorkspace::NewAt(_) = render_loc.workspace else {
            return;
        };

        self.insert_hint_element
            .render(renderer, render_loc.location, &mut |elem| {
                let elem = MonitorInnerRenderElement::UncroppedInsertHint(elem);
                let elem = RescaleRenderElement::from_element(elem, Point::default(), 1.);
                let elem =
                    RelocateRenderElement::from_element(elem, Point::default(), Relocate::Relative);
                push(elem);
            });
    }

    pub fn render_workspaces<R: NiriRenderer>(
        &self,
        mut ctx: RenderCtx<R>,
        focus_ring: bool,
        push: &mut dyn FnMut(MonitorRenderElement<R>),
    ) {
        let _span = tracy_client::span!("Monitor::render_workspaces");

        let scale = self.scale.fractional_scale();
        // Ceil the height in physical pixels.
        let height = (self.view_size.h * scale).ceil() as i32;

        let zoom = self.overview_zoom();

        let insert_hint_render_loc = self
            .insert_hint_render_loc
            .filter(|_| !self.options.layout.insert_hint.off);

        let scale_relocate = move |geo: Rectangle<f64, Logical>, elem| {
            let elem = RescaleRenderElement::from_element(elem, Point::from((0, 0)), zoom);
            RelocateRenderElement::from_element(
                elem,
                // The offset we get from workspaces_with_render_geo() is already
                // rounded to physical pixels, but it's in the logical coordinate
                // space, so we need to convert it to physical.
                geo.loc.to_physical_precise_round(scale),
                Relocate::Relative,
            )
        };

        let pin_geo = self.scratchpad_pin_geo();
        let pin_scratchpad = pin_geo.is_some();

        // Shown scratchpad windows pinned to the screen go on top, uncropped.
        if let Some(geo) = pin_geo {
            let ws = &self.workspaces[self.active_workspace_idx];
            let crop_bounds = Rectangle::new(
                Point::from((-i32::MAX / 2, -i32::MAX / 2)),
                Size::from((i32::MAX, i32::MAX)),
            );
            ws.render_floating(
                ctx.r(),
                XrayPos::new(geo.loc, 1.),
                focus_ring,
                RenderLayer::PinnedScratchpad,
                true,
                &mut |elem| {
                    let elem = CropRenderElement::from_element(elem, scale, crop_bounds);
                    if let Some(elem) = elem {
                        let elem = MonitorInnerRenderElement::from(elem);
                        push(scale_relocate(geo, elem));
                    }
                },
            );
        }

        // Darken the workspaces that are moved out, to go with the skipped workspaces in between.
        // This goes over the workspace contents, but under the pinned scratchpad windows.
        for (buffer, geo) in zip(&self.skip_dim_buffers, &self.skip_dim_geos) {
            let elem = SolidColorRenderElement::from_buffer(
                buffer,
                Point::default(),
                1.,
                Kind::Unspecified,
            );
            let elem = MonitorInnerRenderElement::SolidColor(elem);
            push(scale_relocate(*geo, elem));
        }

        let active_ws_id = self.workspaces[self.active_workspace_idx].id();

        // Draw in passes for correct Z ordering during window movement between workspaces:
        // - floating windows moving between workspaces
        // - normal floating windows
        // - scrolling windows moving between workspaces
        // - normal scrolling windows
        for pass in 0..4 {
            // Don't cull when drawing windows moving between workspaces so that windows moving to
            // workspaces off-screen will still render.
            let cull = matches!(pass, 1 | 3);

            // Crop the elements to prevent them overflowing, currently visible during a workspace
            // switch.
            //
            // HACK: crop to infinite bounds at least horizontally where we
            // know there's no workspace joining or monitor bounds, otherwise
            // it will cut pixel shaders and mess up the coordinate space.
            // There's also a damage tracking bug which causes glitched
            // rendering for maximized GTK windows.
            //
            // FIXME: use proper bounds after fixing the Crop element.
            //
            // Also, check cull here to avoid cropping windows moving between workspaces.
            //
            // FIXME: for cull=true, it might be better visually to crop to a workspace-high region
            // anchored to the window/column as it moves between workspaces, to prevent overflowing
            // windows from appearing and disappearing.
            let crop_bounds =
                if cull && (self.workspace_switch.is_some() || self.overview_progress.is_some()) {
                    Rectangle::new(
                        Point::from((-i32::MAX / 2, 0)),
                        Size::from((i32::MAX, height)),
                    )
                } else {
                    Rectangle::new(
                        Point::from((-i32::MAX / 2, -i32::MAX / 2)),
                        Size::from((i32::MAX, i32::MAX)),
                    )
                };

            for (ws, geo, clip) in self.workspaces_with_render_geo_and_clip_cull(cull) {
                // Cut off the part of the workspace covered by others during a skipping switch.
                // This only happens outside the overview, so there's no zoom to account for.
                let crop_bounds = match clip {
                    Some(clip) if cull => {
                        let top = ((clip.loc.y - geo.loc.y) * scale).round() as i32;
                        let bottom =
                            ((clip.loc.y + clip.size.h - geo.loc.y) * scale).round() as i32;
                        let top = top.max(crop_bounds.loc.y);
                        let bottom = bottom.min(crop_bounds.loc.y + crop_bounds.size.h);
                        Rectangle::new(
                            Point::from((crop_bounds.loc.x, top)),
                            Size::from((crop_bounds.size.w, (bottom - top).max(0))),
                        )
                    }
                    _ => crop_bounds,
                };

                // Macro instead of closure because ws and insert hint have different elem types.
                macro_rules! push {
                    () => {{
                        &mut |elem| {
                            let elem = CropRenderElement::from_element(elem, scale, crop_bounds);
                            if let Some(elem) = elem {
                                let elem = MonitorInnerRenderElement::from(elem);
                                push(scale_relocate(geo, elem));
                            }
                        }
                    }};
                }

                let xray_pos = XrayPos::new(geo.loc, zoom);

                match pass {
                    0 => {
                        ws.render_floating(
                            ctx.r(),
                            xray_pos,
                            focus_ring,
                            RenderLayer::MovingBetweenWorkspaces,
                            pin_scratchpad,
                            push!(),
                        );
                    }
                    1 => {
                        // Only the active workspace has anything pinned.
                        let pin_scratchpad = pin_scratchpad && ws.id() == active_ws_id;
                        ws.render_floating(
                            ctx.r(),
                            xray_pos,
                            focus_ring,
                            RenderLayer::Normal,
                            pin_scratchpad,
                            push!(),
                        );

                        if let Some(loc) = insert_hint_render_loc {
                            if loc.workspace == InsertWorkspace::Existing(ws.id()) {
                                self.insert_hint_element.render(
                                    ctx.renderer,
                                    loc.location,
                                    push!(),
                                );
                            }
                        }
                    }
                    2 => {
                        ws.render_scrolling(
                            ctx.r(),
                            xray_pos,
                            focus_ring,
                            RenderLayer::MovingBetweenWorkspaces,
                            push!(),
                        );
                    }
                    _ => {
                        ws.render_scrolling(
                            ctx.r(),
                            xray_pos,
                            focus_ring,
                            RenderLayer::Normal,
                            push!(),
                        );
                    }
                }
            }
        }
    }

    pub fn render_workspace_shadows<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        push: &mut dyn FnMut(MonitorRenderElement<R>),
    ) {
        let Some(progress) = self.overview_progress.as_ref().map(|p| p.clamped_value()) else {
            return;
        };
        let alpha = progress.clamp(0., 1.) as f32;

        let _span = tracy_client::span!("Monitor::render_workspace_shadows");

        let scale = self.scale.fractional_scale();
        let zoom = self.overview_zoom();

        for (ws, geo) in self.workspaces_with_render_geo() {
            ws.render_shadow(renderer, &mut |elem| {
                let elem = elem.with_alpha(alpha);
                let elem = MonitorInnerRenderElement::Shadow(elem);
                let elem = RescaleRenderElement::from_element(elem, Point::from((0, 0)), zoom);
                let elem = RelocateRenderElement::from_element(
                    elem,
                    geo.loc.to_physical_precise_round(scale),
                    Relocate::Relative,
                );
                push(elem);
            });
        }
    }

    pub fn workspace_switch_gesture_begin(&mut self, is_touchpad: bool) {
        let center_idx = self.active_workspace_idx;
        let current_idx = self.workspace_render_idx();

        let gesture = WorkspaceSwitchGesture {
            center_idx,
            start_idx: current_idx,
            current_idx,
            animation: None,
            tracker: SwipeTracker::new(),
            is_touchpad,
            is_clamped: !self.overview_open,
            dnd_last_event_time: None,
            dnd_nonzero_start_time: None,
        };
        self.workspace_switch = Some(WorkspaceSwitch::Gesture(gesture));
    }

    pub fn dnd_scroll_gesture_begin(&mut self) {
        if let Some(WorkspaceSwitch::Gesture(WorkspaceSwitchGesture {
            dnd_last_event_time: Some(_),
            ..
        })) = &self.workspace_switch
        {
            // Already active.
            return;
        }

        if !self.overview_open {
            // This gesture is only for the overview.
            return;
        }

        let center_idx = self.active_workspace_idx;
        let current_idx = self.workspace_render_idx();

        let gesture = WorkspaceSwitchGesture {
            center_idx,
            start_idx: current_idx,
            current_idx,
            animation: None,
            tracker: SwipeTracker::new(),
            is_touchpad: false,
            is_clamped: false,
            dnd_last_event_time: Some(self.clock.now_unadjusted()),
            dnd_nonzero_start_time: None,
        };
        self.workspace_switch = Some(WorkspaceSwitch::Gesture(gesture));
    }

    pub fn workspace_switch_gesture_update(
        &mut self,
        delta_y: f64,
        timestamp: Duration,
        is_touchpad: bool,
    ) -> Option<bool> {
        let Some(WorkspaceSwitch::Gesture(gesture)) = &self.workspace_switch else {
            return None;
        };

        if gesture.is_touchpad != is_touchpad || gesture.dnd_last_event_time.is_some() {
            return None;
        }

        let zoom = self.overview_zoom();
        let total_height = if gesture.is_touchpad {
            WORKSPACE_GESTURE_MOVEMENT
        } else {
            self.workspace_size_with_gap(1.).h
        };

        let Some(WorkspaceSwitch::Gesture(gesture)) = &mut self.workspace_switch else {
            return None;
        };

        // Reduce the effect of zoom on the touchpad somewhat.
        let delta_scale = if gesture.is_touchpad {
            (zoom - 1.) / 2.5 + 1.
        } else {
            zoom
        };

        let delta_y = delta_y / delta_scale;
        let mut rubber_band = WORKSPACE_GESTURE_RUBBER_BAND;
        rubber_band.limit /= zoom;

        gesture.tracker.push(delta_y, timestamp);

        let pos = gesture.tracker.pos() / total_height;

        let (min, max) = gesture.min_max(self.workspaces.len());
        let new_idx = gesture.start_idx + pos;
        let new_idx = rubber_band.clamp(min, max, new_idx);

        if gesture.current_idx == new_idx {
            return Some(false);
        }

        gesture.current_idx = new_idx;
        Some(true)
    }

    pub fn dnd_scroll_gesture_scroll(&mut self, pos: Point<f64, Logical>, speed: f64) -> bool {
        let zoom = self.overview_zoom();

        let Some(WorkspaceSwitch::Gesture(gesture)) = &mut self.workspace_switch else {
            return false;
        };

        let Some(last_time) = gesture.dnd_last_event_time else {
            // Not a DnD scroll.
            return false;
        };

        let config = &self.options.gestures.dnd_edge_workspace_switch;
        let trigger_height = config.trigger_height;

        // Restrict the scrolling horizontally to the strip of workspaces to avoid unwanted trigger
        // after using the hot corner or during horizontal scroll.
        let width = self.view_size.w * zoom;
        let x = pos.x - (self.view_size.w - width) / 2.;

        // Consider the working area so layer-shell docks and such don't prevent scrolling.
        let y = pos.y - self.working_area.loc.y;
        let height = self.working_area.size.h;

        let y = y.clamp(0., height);
        let trigger_height = trigger_height.clamp(0., height / 2.);

        let delta = if x < 0. || width <= x {
            // Outside the bounds horizontally.
            0.
        } else if y < trigger_height {
            -(trigger_height - y)
        } else if height - y < trigger_height {
            trigger_height - (height - y)
        } else {
            0.
        };

        let delta = if trigger_height < 0.01 {
            // Sanity check for trigger-height 0 or small window sizes.
            0.
        } else {
            // Normalize to [0, 1].
            delta / trigger_height
        };
        let delta = delta * speed;

        let now = self.clock.now_unadjusted();
        gesture.dnd_last_event_time = Some(now);

        if delta == 0. {
            // We're outside the scrolling zone.
            gesture.dnd_nonzero_start_time = None;
            return false;
        }

        let nonzero_start = *gesture.dnd_nonzero_start_time.get_or_insert(now);

        // Delay starting the gesture a bit to avoid unwanted movement when dragging across
        // monitors.
        let delay = Duration::from_millis(u64::from(config.delay_ms));
        if now.saturating_sub(nonzero_start) < delay {
            return true;
        }

        let time_delta = now.saturating_sub(last_time).as_secs_f64();

        let delta = delta * time_delta * config.max_speed;

        gesture.tracker.push(delta, now);

        let total_height = WORKSPACE_DND_EDGE_SCROLL_MOVEMENT;
        let pos = gesture.tracker.pos() / total_height;
        let unclamped = gesture.start_idx + pos;

        let (min, max) = gesture.min_max(self.workspaces.len());
        let clamped = unclamped.clamp(min, max);

        // Make sure that DnD scrolling too much outside the min/max does not "build up".
        gesture.start_idx += clamped - unclamped;
        gesture.current_idx = clamped;

        true
    }

    pub fn workspace_switch_gesture_end(&mut self, is_touchpad: Option<bool>) -> bool {
        let Some(WorkspaceSwitch::Gesture(gesture)) = &self.workspace_switch else {
            return false;
        };

        if is_touchpad.is_some_and(|x| gesture.is_touchpad != x) {
            return false;
        }

        let zoom = self.overview_zoom();
        let total_height = if gesture.dnd_last_event_time.is_some() {
            WORKSPACE_DND_EDGE_SCROLL_MOVEMENT
        } else if gesture.is_touchpad {
            WORKSPACE_GESTURE_MOVEMENT
        } else {
            self.workspace_size_with_gap(1.).h
        };

        let Some(WorkspaceSwitch::Gesture(gesture)) = &mut self.workspace_switch else {
            return false;
        };

        // Take into account any idle time between the last event and now.
        let now = self.clock.now_unadjusted();
        gesture.tracker.push(0., now);

        let mut rubber_band = WORKSPACE_GESTURE_RUBBER_BAND;
        rubber_band.limit /= zoom;

        let mut velocity = gesture.tracker.velocity() / total_height;
        let current_pos = gesture.tracker.pos() / total_height;
        let pos = gesture.tracker.projected_end_pos() / total_height;

        let (min, max) = gesture.min_max(self.workspaces.len());
        let new_idx = gesture.start_idx + pos;

        let new_idx = new_idx.clamp(min, max);
        let new_idx = new_idx.round() as usize;

        velocity *= rubber_band.clamp_derivative(min, max, gesture.start_idx + current_pos);

        if self.active_workspace_idx != new_idx {
            self.previous_workspace_id = Some(self.workspaces[self.active_workspace_idx].id());
        }

        self.active_workspace_idx = new_idx;
        self.workspace_switch = Some(WorkspaceSwitch::Animation(Animation::new(
            self.clock.clone(),
            gesture.current_idx,
            new_idx as f64,
            velocity,
            self.options.animations.workspace_switch.0,
        )));

        self.scratchpad_follow();

        true
    }

    pub fn dnd_scroll_gesture_end(&mut self) {
        if !matches!(
            self.workspace_switch,
            Some(WorkspaceSwitch::Gesture(WorkspaceSwitchGesture {
                dnd_last_event_time: Some(_),
                ..
            }))
        ) {
            // Not a DnD scroll.
            return;
        };

        self.workspace_switch_gesture_end(None);
    }

    pub fn scale(&self) -> smithay::output::Scale {
        self.scale
    }

    pub fn view_size(&self) -> Size<f64, Logical> {
        self.view_size
    }

    pub fn working_area(&self) -> Rectangle<f64, Logical> {
        self.working_area
    }

    pub fn layout_config(&self) -> Option<&niri_config::LayoutPart> {
        self.layout_config.as_ref()
    }

    #[cfg(test)]
    pub(super) fn verify_invariants(&self) {
        use approx::assert_abs_diff_eq;

        let options =
            Options::clone(&self.base_options).with_merged_layout(self.layout_config.as_ref());
        assert_eq!(&*self.options, &options);

        assert!(
            !self.workspaces.is_empty(),
            "monitor must have at least one workspace"
        );
        assert!(self.active_workspace_idx < self.workspaces.len());

        if let Some(WorkspaceSwitch::Animation(anim)) = &self.workspace_switch {
            let before_idx = anim.from() as usize;
            let after_idx = anim.to() as usize;

            assert!(before_idx < self.workspaces.len());
            assert!(after_idx < self.workspaces.len());
        }

        if let Some(WorkspaceSwitch::Skip(skip)) = &self.workspace_switch {
            assert!(skip.target_idx >= 0.);
            assert!((skip.target_idx as usize) < self.workspaces.len());
            assert_eq!(skip.target_idx as usize, self.active_workspace_idx);

            // The target is always one of the items, and nothing's shown twice.
            let target_id = self.workspaces[skip.target_idx as usize].id();
            assert!(skip.items.iter().any(|item| item.id == target_id));
            for (i, item) in skip.items.iter().enumerate() {
                assert!(skip.items[i + 1..].iter().all(|other| other.id != item.id));
            }
        }

        assert!(
            !self.workspaces.last().unwrap().has_windows(),
            "monitor must have an empty workspace in the end"
        );
        if self.options.layout.empty_workspace_above_first {
            assert!(
                !self.workspaces.first().unwrap().has_windows(),
                "first workspace must be empty when empty_workspace_above_first is set"
            )
        }

        assert!(
            self.workspaces.last().unwrap().name.is_none(),
            "monitor must have an unnamed workspace in the end"
        );
        if self.options.layout.empty_workspace_above_first {
            assert!(
                self.workspaces.first().unwrap().name.is_none(),
                "first workspace must be unnamed when empty_workspace_above_first is set"
            )
        }

        if self.options.layout.empty_workspace_above_first {
            assert!(
                self.workspaces.len() != 2,
                "if empty_workspace_above_first is set there must be just 1 or 3+ workspaces"
            )
        }

        // If there's no workspace switch in progress, there can't be any non-last non-active
        // empty workspaces. If empty_workspace_above_first is set then the first workspace
        // will be empty too.
        let pre_skip = if self.options.layout.empty_workspace_above_first {
            1
        } else {
            0
        };
        if self.workspace_switch.is_none() {
            for (idx, ws) in self
                .workspaces
                .iter()
                .enumerate()
                .skip(pre_skip)
                .rev()
                // skip last
                .skip(1)
            {
                if idx != self.active_workspace_idx {
                    assert!(
                        ws.has_windows_or_name(),
                        "non-active workspace can't be empty and unnamed except the last one"
                    );
                }
            }
        }

        for workspace in &self.workspaces {
            assert_eq!(self.clock, workspace.clock);

            assert_eq!(
                self.scale().integer_scale(),
                workspace.scale().integer_scale()
            );
            assert_eq!(
                self.scale().fractional_scale(),
                workspace.scale().fractional_scale()
            );
            assert_eq!(self.view_size, workspace.view_size());
            assert_eq!(self.working_area, workspace.working_area());

            assert_eq!(
                workspace.base_options, self.options,
                "workspace options must be synchronized with monitor"
            );
        }

        let scale = self.scale().fractional_scale();
        let iter = self.workspaces_with_render_geo();
        for (_ws, ws_geo) in iter {
            let pos = ws_geo.loc;
            let rounded_pos = pos.to_physical_precise_round(scale).to_logical(scale);

            // Workspace positions must be rounded to physical pixels.
            assert_abs_diff_eq!(pos.x, rounded_pos.x, epsilon = 1e-5);
            assert_abs_diff_eq!(pos.y, rounded_pos.y, epsilon = 1e-5);
        }
    }
}

/// Returns whether the workspace with render geometry `geo` and visible part `clip` shows up on the
/// output.
fn is_on_output(
    geo: Rectangle<f64, Logical>,
    clip: Option<Rectangle<f64, Logical>>,
    output_geo: Rectangle<f64, Logical>,
) -> bool {
    let visible = clip.unwrap_or(geo);
    visible.size.h > 0. && visible.intersection(output_geo).is_some()
}
