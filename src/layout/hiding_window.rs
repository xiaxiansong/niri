use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Scale, Size};

use super::opening_window::{OpenAnimation, OpeningWindowRenderElement};
use super::tile::TileRenderSnapshot;
use crate::animation::Animation;
use crate::render_helpers::{RenderCtx, RenderTarget};

/// A window being hidden to the scratchpad, animating out.
///
/// The animation is the window opening animation played backwards, so that showing the window
/// again can continue from the current progress.
#[derive(Debug)]
pub struct HidingWindow<Id> {
    /// Id of the hidden window.
    pub id: Id,

    /// Generation of the hiding, matching the hidden tile's `scratchpad_hide_gen`.
    pub gen: u64,

    /// The animation, going from the opening progress down to 0.
    anim: OpenAnimation,

    /// Snapshot of the tile, filled in later since the layout has no renderer.
    snapshot: Option<TileRenderSnapshot>,

    /// Render position of the tile in the workspace.
    pos: Point<f64, Logical>,

    /// Size of the tile.
    tile_size: Size<f64, Logical>,
}

impl<Id> HidingWindow<Id> {
    pub fn new(
        id: Id,
        gen: u64,
        anim: Animation,
        pos: Point<f64, Logical>,
        tile_size: Size<f64, Logical>,
    ) -> Self {
        Self {
            id,
            gen,
            anim: OpenAnimation::new(anim),
            snapshot: None,
            pos,
            tile_size,
        }
    }

    pub fn is_done(&self) -> bool {
        self.anim.is_done()
    }

    pub fn pos(&self) -> Point<f64, Logical> {
        self.pos
    }

    pub fn has_snapshot(&self) -> bool {
        self.snapshot.is_some()
    }

    pub fn set_snapshot(&mut self, snapshot: TileRenderSnapshot) {
        self.snapshot = Some(snapshot);
    }

    pub fn render(
        &self,
        ctx: RenderCtx<GlesRenderer>,
        scale: Scale<f64>,
    ) -> Option<OpeningWindowRenderElement> {
        let snapshot = self.snapshot.as_ref()?;

        let contents = if ctx.target.should_block_out(snapshot.block_out_from) {
            &snapshot.blocked_out_contents
        } else if ctx.target != RenderTarget::Output
            && snapshot.contents_with_blocked_out_bg.is_some()
        {
            snapshot.contents_with_blocked_out_bg.as_ref().unwrap()
        } else {
            &snapshot.contents
        };

        match self
            .anim
            .render(ctx.renderer, contents, self.tile_size, self.pos, scale, 1.)
        {
            Ok((elem, _data)) => Some(elem),
            Err(err) => {
                warn!("error rendering window hiding animation: {err:?}");
                None
            }
        }
    }
}
