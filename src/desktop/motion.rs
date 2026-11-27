use gpui::{
    Animation, AnimationExt, AnyElement, App, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Rgba, Style, Styled, Window, div,
    ease_out_quint, fill, prelude::*, px, rgba,
};
use std::time::{Duration, Instant};

const ENTRANCE: Duration = Duration::from_millis(160);
const DIALOG_ENTRANCE: Duration = Duration::from_millis(220);
const DURATION: Duration = Duration::from_millis(180);

pub(super) fn reveal(
    element: impl Styled + IntoElement + 'static,
    id: impl Into<ElementId>,
    enabled: bool,
) -> AnyElement {
    enter(element, id, enabled, ENTRANCE, 0.72, 0.)
}

pub(super) fn reveal_dialog(
    element: impl Styled + IntoElement + 'static,
    id: &'static str,
    enabled: bool,
) -> AnyElement {
    enter(element, id, enabled, DIALOG_ENTRANCE, 0.72, 8.)
}

pub(super) fn backdrop(enabled: bool) -> AnyElement {
    enter(
        div().absolute().inset_0().bg(rgba(0x17130f80)),
        "dialog-backdrop",
        enabled,
        ENTRANCE,
        0.,
        0.,
    )
}

fn enter(
    element: impl Styled + IntoElement + 'static,
    id: impl Into<ElementId>,
    enabled: bool,
    duration: Duration,
    opacity: f32,
    travel: f32,
) -> AnyElement {
    if !enabled {
        return element.into_any_element();
    }
    element
        .with_animation(
            id,
            Animation::new(duration).with_easing(ease_out_quint()),
            move |el, value| {
                el.opacity(opacity + (1. - opacity) * value)
                    .when(travel > 0., |el| {
                        el.relative().top(px(travel * (1. - value)))
                    })
            },
        )
        .into_any_element()
}

pub(super) fn pending(color: Rgba, enabled: bool) -> AnyElement {
    let dot = div().size(px(7.)).flex_none().rounded_full().bg(color);
    if !enabled {
        return dot.into_any_element();
    }
    dot.with_animation(
        "pending-metadata",
        Animation::new(Duration::from_millis(600)),
        |dot, value| dot.opacity(1. - 0.45 * (std::f32::consts::PI * value).sin()),
    )
    .into_any_element()
}

#[derive(Clone, Copy)]
struct ProgressMotion {
    from: f32,
    target: f32,
    start: Instant,
}

impl ProgressMotion {
    fn value(self, now: Instant) -> f32 {
        let elapsed = now.duration_since(self.start);
        if elapsed >= DURATION {
            return self.target;
        }
        let fraction = elapsed.as_secs_f32() / DURATION.as_secs_f32();
        self.from + (self.target - self.from) * ease_out_quint()(fraction)
    }

    fn retarget(self, target: f32, now: Instant) -> Self {
        if target == self.target {
            return self;
        }
        Self {
            from: self.value(now),
            target,
            start: now,
        }
    }
}

pub(super) struct ProgressTrack {
    pub id: i64,
    pub height: Pixels,
    pub value: f32,
    pub fill: Rgba,
    pub track: Rgba,
    pub animate: bool,
}

impl IntoElement for ProgressTrack {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for ProgressTrack {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(("progress-track", self.id as usize).into())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.height = self.height.into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        mut bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) {
        let radius = bounds.size.height / 2.;
        window.paint_quad(fill(bounds.clone(), self.track).corner_radii(radius));
        let target = if self.value.is_finite() {
            self.value.clamp(0., 1.)
        } else {
            0.
        };
        let now = Instant::now();
        let initial = ProgressMotion {
            from: target,
            target,
            start: now,
        };
        let value =
            window.with_element_state(id.unwrap(), |previous: Option<ProgressMotion>, window| {
                let motion = if self.animate && window.is_window_active() {
                    previous.unwrap_or(initial).retarget(target, now)
                } else {
                    initial
                };
                if motion.from != motion.target && now.duration_since(motion.start) < DURATION {
                    window.request_animation_frame();
                }
                (motion.value(now), motion)
            });
        if value > 0. {
            bounds.size.width *= value;
            window.paint_quad(fill(bounds, self.fill).corner_radii(radius));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_retargets_from_the_displayed_value() {
        let now = Instant::now();
        let motion = ProgressMotion {
            from: 0.,
            target: 0.5,
            start: now,
        };
        let middle = now + DURATION / 2;
        let displayed = motion.value(middle);
        assert!(displayed > 0. && displayed < 0.5);
        let next = motion.retarget(0.9, middle);
        assert_eq!(next.value(middle), displayed);
        assert_eq!(next.retarget(0.9, middle + DURATION / 4).start, middle);
        assert_eq!(next.value(middle + DURATION), 0.9);
        assert_eq!(
            next.retarget(0.1, middle + DURATION)
                .value(middle + DURATION),
            0.9
        );
    }
}
