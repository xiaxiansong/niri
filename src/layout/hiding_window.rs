use super::opening_window::OpenAnimation;
use crate::animation::Animation;

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
}

impl<Id> HidingWindow<Id> {
    pub fn new(id: Id, gen: u64, anim: Animation) -> Self {
        Self {
            id,
            gen,
            anim: OpenAnimation::new(anim),
        }
    }

    pub fn is_done(&self) -> bool {
        self.anim.is_done()
    }
}
