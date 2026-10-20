//! Wayland bridge for the tablet and the trackpad.
//!
//! winit 0.30 does not listen to `zwp_tablet_manager_v2` or to
//! `zwp_pointer_gestures_v1` (Linux). We attach to eframe's
//! `wl_display` (guest): stylus tip and pressure, and two-finger
//! pinch zoom (armed after a small scale deadzone) with pan.
//! Three-finger swipe is left to the compositor (Omarchy workspaces).

/// Scale change from pinch begin before zoom engages (noise / scroll false starts).
/// Very small: waiting here felt like latency before the page reacted.
const PINCH_ZOOM_ARM: f32 = 0.003;
/// Linear boost on each frame's scale ratio.
const PINCH_ZOOM_GAIN: f32 = 2.0;
/// Cap one Wayland update so a batched first event cannot jump the camera.
const PINCH_ZOOM_FRAME_MIN: f32 = 0.88;
const PINCH_ZOOM_FRAME_MAX: f32 = 1.14;
/// Shared (dx, dy) weight before zoom has armed — hold the page still while
/// distance is still building (typical of one-finger-moving pinches).
const PINCH_PAN_BEFORE_ZOOM: f32 = 0.08;
/// Shared (dx, dy) weight while a zoom frame is applied.
const PINCH_PAN_WHILE_ZOOM: f32 = 0.12;

/// Frame zoom factor from a Wayland pinch `scale` (1.0 = none this frame).
///
/// Wayland `scale` is finger distance vs begin, so a pivot pinch (one finger
/// still, the other approaching or leaving) is the same signal as a symmetric
/// pinch. We arm on a small distance change, ease in from begin with linear
/// gain (no powered dump), and clamp each frame for continuous tracking.
fn pinch_frame_zoom(armed: &mut bool, last_scale: &mut f32, scale: f32, arm: f32) -> f32 {
    if !*armed {
        if (scale - 1.0).abs() < arm {
            return 1.0;
        }
        *armed = true;
        // Ease from the true begin so the first real distance change is felt,
        // without the old ^power cliff on a batched update.
        *last_scale = 1.0;
    }
    if *last_scale <= 0.05 {
        *last_scale = scale;
        return 1.0;
    }
    let raw = (scale / *last_scale).max(0.05);
    *last_scale = scale;
    (1.0 + (raw - 1.0) * PINCH_ZOOM_GAIN).clamp(PINCH_ZOOM_FRAME_MIN, PINCH_ZOOM_FRAME_MAX)
}

use std::collections::HashSet;

use egui::{Pos2, Vec2};
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use wayland_backend::client::Backend;
use wayland_client::protocol::{
    wl_pointer::{self, WlPointer},
    wl_registry::{self, WlRegistry},
    wl_seat::{self, WlSeat},
};
use wayland_client::{event_created_child, Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::wp::pointer_gestures::zv1::client::{
    zwp_pointer_gesture_pinch_v1::{self, ZwpPointerGesturePinchV1},
    zwp_pointer_gestures_v1::{self, ZwpPointerGesturesV1},
};
use wayland_protocols::wp::tablet::zv2::client::{
    zwp_tablet_manager_v2::{self, ZwpTabletManagerV2},
    zwp_tablet_pad_dial_v2::{self, ZwpTabletPadDialV2},
    zwp_tablet_pad_group_v2::{self, ZwpTabletPadGroupV2},
    zwp_tablet_pad_ring_v2::{self, ZwpTabletPadRingV2},
    zwp_tablet_pad_strip_v2::{self, ZwpTabletPadStripV2},
    zwp_tablet_pad_v2::{self, ZwpTabletPadV2},
    zwp_tablet_seat_v2::{self, ZwpTabletSeatV2},
    zwp_tablet_tool_v2::{self, ZwpTabletToolV2},
    zwp_tablet_v2::{self, ZwpTabletV2},
};

const BTN_STYLUS: u32 = 0x14b;
const BTN_STYLUS2: u32 = 0x14c;

#[derive(Clone, Debug)]
pub struct PenSnapshot {
    pub pos: Option<Pos2>,
    pub samples: Vec<Pos2>,
    pub down: bool,
    pub pressed: bool,
    pub released: bool,
    pub eraser: bool,
    /// Button 2 held: lasso for the length of the gesture.
    pub lasso_btn: bool,
    /// Button 1 click in the air (in proximity, tip not down).
    pub air_toggle: bool,
    pub pressure: Option<f32>,
    pub in_proximity: bool,
    /// Trackpad two-finger pinch zoom factor (1.0 = none), relative to this frame.
    pub pinch_zoom: f32,
    pub pinch_pan: Vec2,
    pub pinching: bool,
}

impl Default for PenSnapshot {
    fn default() -> Self {
        Self {
            pos: None,
            samples: Vec::new(),
            down: false,
            pressed: false,
            released: false,
            eraser: false,
            lasso_btn: false,
            air_toggle: false,
            pressure: None,
            in_proximity: false,
            pinch_zoom: 1.0,
            pinch_pan: Vec2::ZERO,
            pinching: false,
        }
    }
}

pub struct TabletBridge {
    inner: Option<Inner>,
    dead: bool,
    /// Test-only stylus override: real Wayland state is unreachable in tests.
    #[cfg(test)]
    test_pen: Option<PenSnapshot>,
}

struct Inner {
    conn: Connection,
    queue: EventQueue<TabletState>,
    state: TabletState,
}

#[derive(Default)]
struct TabletState {
    #[allow(dead_code)]
    registry: Option<WlRegistry>,
    manager: Option<ZwpTabletManagerV2>,
    seats: Vec<(u32, WlSeat)>,
    #[allow(dead_code)]
    tablet_seats: Vec<ZwpTabletSeatV2>,
    bound_seats: HashSet<u32>,
    pos: Option<Pos2>,
    samples: Vec<Pos2>,
    down: bool,
    pressed: bool,
    released: bool,
    in_proximity: bool,
    eraser_tool: bool,
    stylus_btn: bool,
    stylus2_btn: bool,
    /// Down seen while button 1 was held, so this is a tip press and not an air click.
    stylus_btn_saw_down: bool,
    air_toggle: bool,
    pressure: Option<f32>,
    gestures: Option<ZwpPointerGesturesV1>,
    #[allow(dead_code)]
    pointers: Vec<WlPointer>,
    #[allow(dead_code)]
    pinches: Vec<ZwpPointerGesturePinchV1>,
    pinch_bound: HashSet<u32>,
    pinch_last_scale: f32,
    pinch_zoom_armed: bool,
    pinch_zoom: f32,
    pinch_pan: Vec2,
    pinching: bool,
}

impl TabletBridge {
    pub fn new() -> Self {
        Self {
            inner: None,
            dead: false,
            #[cfg(test)]
            test_pen: None,
        }
    }

    /// Drive the stylus snapshot without Wayland (tests only).
    #[cfg(test)]
    pub(crate) fn set_test_pen(&mut self, pen: Option<PenSnapshot>) {
        self.test_pen = pen;
    }

    pub fn ready(&self) -> bool {
        self.inner.is_some() && !self.dead
    }

    /// Attaches the `wl_display` (once). Call from `update` with the `Frame`.
    pub fn ensure(&mut self, frame: &eframe::Frame) {
        if self.dead || self.inner.is_some() {
            return;
        }
        match unsafe { attach(frame) } {
            Ok(inner) => self.inner = Some(inner),
            Err(()) => self.dead = true,
        }
    }

    /// Reads Wayland events. Call from `raw_input_hook` (before egui).
    pub fn pump_events(&mut self) {
        if self.dead {
            return;
        }
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        inner.state.pressed = false;
        inner.state.released = false;
        inner.state.air_toggle = false;
        inner.state.samples.clear();
        inner.state.pinch_zoom = 1.0;
        inner.state.pinch_pan = Vec2::ZERO;
        let _ = inner.conn.flush();
        if inner.queue.dispatch_pending(&mut inner.state).is_err() {
            self.dead = true;
            self.inner = None;
            return;
        }
        let _ = inner.conn.flush();
    }

    pub fn snapshot(&self) -> PenSnapshot {
        #[cfg(test)]
        if let Some(pen) = &self.test_pen {
            return pen.clone();
        }
        let Some(inner) = self.inner.as_ref() else {
            return PenSnapshot::default();
        };
        let s = &inner.state;
        PenSnapshot {
            pos: s.pos,
            samples: s.samples.clone(),
            down: s.down,
            pressed: s.pressed,
            released: s.released,
            eraser: s.eraser_tool || s.stylus_btn,
            lasso_btn: s.stylus2_btn,
            air_toggle: s.air_toggle,
            pressure: s.pressure,
            in_proximity: s.in_proximity,
            pinch_zoom: s.pinch_zoom,
            pinch_pan: s.pinch_pan,
            pinching: s.pinching,
        }
    }

    pub fn wants_repaint(&self) -> bool {
        if self.dead {
            return false;
        }
        match &self.inner {
            None => true,
            Some(i) => i.state.in_proximity || i.state.down || i.state.pinching,
        }
    }
}

unsafe fn attach(frame: &eframe::Frame) -> Result<Inner, ()> {
    let handle = frame.display_handle().map_err(|_| ())?;
    let RawDisplayHandle::Wayland(w) = handle.as_raw() else {
        return Err(());
    };
    let backend = unsafe { Backend::from_foreign_display(w.display.as_ptr().cast()) };
    let conn = Connection::from_backend(backend);
    let mut queue: EventQueue<TabletState> = conn.new_event_queue();
    let qh = queue.handle();
    let registry = conn.display().get_registry(&qh, ());
    let mut state = TabletState {
        registry: Some(registry),
        ..Default::default()
    };
    let _ = conn.flush();
    let _ = queue.dispatch_pending(&mut state);
    Ok(Inner { conn, queue, state })
}

impl TabletState {
    fn bind_tablet_seats(&mut self, qh: &QueueHandle<Self>) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        for (name, seat) in &self.seats {
            if !self.bound_seats.insert(*name) {
                continue;
            }
            self.tablet_seats
                .push(manager.get_tablet_seat(seat, qh, ()));
        }
    }

    fn bind_gestures(&mut self, qh: &QueueHandle<Self>) {
        let Some(gestures) = self.gestures.clone() else {
            return;
        };
        for (name, seat) in &self.seats {
            if !self.pinch_bound.insert(*name) {
                continue;
            }
            let pointer = seat.get_pointer(qh, ());
            self.pinches
                .push(gestures.get_pinch_gesture(&pointer, qh, ()));
            self.pointers.push(pointer);
        }
    }
}

fn tool_pos(x: f64, y: f64) -> Pos2 {
    Pos2::new(x as f32, y as f32)
}

impl Dispatch<WlRegistry, ()> for TabletState {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" => {
                    let seat: WlSeat = registry.bind(name, version.min(1), qh, ());
                    state.seats.push((name, seat));
                    state.bind_tablet_seats(qh);
                    state.bind_gestures(qh);
                }
                "zwp_tablet_manager_v2" | "wp_tablet_manager_v2" => {
                    if state.manager.is_none() {
                        state.manager = Some(registry.bind(name, version.min(2), qh, ()));
                        state.bind_tablet_seats(qh);
                    }
                }
                "zwp_pointer_gestures_v1" if state.gestures.is_none() => {
                    state.gestures = Some(registry.bind(name, version.min(1), qh, ()));
                    state.bind_gestures(qh);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for TabletState {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlPointer, ()> for TabletState {
    fn event(
        _: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let _ = event;
    }
}

impl Dispatch<ZwpPointerGesturesV1, ()> for TabletState {
    fn event(
        _: &mut Self,
        _: &ZwpPointerGesturesV1,
        event: zwp_pointer_gestures_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let _ = event;
    }
}

impl Dispatch<ZwpPointerGesturePinchV1, ()> for TabletState {
    fn event(
        state: &mut Self,
        _: &ZwpPointerGesturePinchV1,
        event: zwp_pointer_gesture_pinch_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_pointer_gesture_pinch_v1::Event::Begin { fingers, .. } => {
                if fingers >= 2 {
                    state.pinching = true;
                    state.pinch_last_scale = 1.0;
                    state.pinch_zoom_armed = false;
                }
            }
            zwp_pointer_gesture_pinch_v1::Event::Update { dx, dy, scale, .. } => {
                let scale = scale as f32;
                state.pinching = true;
                let zoom = pinch_frame_zoom(
                    &mut state.pinch_zoom_armed,
                    &mut state.pinch_last_scale,
                    scale,
                    PINCH_ZOOM_ARM,
                );
                state.pinch_zoom *= zoom;
                // Pivot pinches move the logical center a lot while distance
                // changes; keep that common-mode pan quiet until / while zooming.
                let pan_w = if !state.pinch_zoom_armed {
                    PINCH_PAN_BEFORE_ZOOM
                } else if (zoom - 1.0).abs() > 0.0005 {
                    PINCH_PAN_WHILE_ZOOM
                } else {
                    1.0
                };
                state.pinch_pan += Vec2::new(dx as f32, dy as f32) * pan_w;
            }
            zwp_pointer_gesture_pinch_v1::Event::End { .. } => {
                state.pinching = false;
                state.pinch_last_scale = 0.0;
                state.pinch_zoom_armed = false;
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpTabletManagerV2, ()> for TabletState {
    fn event(
        _: &mut Self,
        _: &ZwpTabletManagerV2,
        event: zwp_tablet_manager_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let _ = event;
    }
}

impl Dispatch<ZwpTabletSeatV2, ()> for TabletState {
    fn event(
        _: &mut Self,
        _: &ZwpTabletSeatV2,
        _: zwp_tablet_seat_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(TabletState, ZwpTabletSeatV2, [
        zwp_tablet_seat_v2::EVT_TABLET_ADDED_OPCODE => (ZwpTabletV2, ()),
        zwp_tablet_seat_v2::EVT_TOOL_ADDED_OPCODE => (ZwpTabletToolV2, ()),
        zwp_tablet_seat_v2::EVT_PAD_ADDED_OPCODE => (ZwpTabletPadV2, ()),
    ]);
}

impl Dispatch<ZwpTabletV2, ()> for TabletState {
    fn event(
        _: &mut Self,
        tablet: &ZwpTabletV2,
        event: zwp_tablet_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, zwp_tablet_v2::Event::Removed) {
            tablet.destroy();
        }
    }
}

impl Dispatch<ZwpTabletToolV2, ()> for TabletState {
    fn event(
        state: &mut Self,
        tool: &ZwpTabletToolV2,
        event: zwp_tablet_tool_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_tablet_tool_v2::Event::Type { tool_type } => {
                state.eraser_tool =
                    matches!(tool_type, WEnum::Value(zwp_tablet_tool_v2::Type::Eraser));
            }
            zwp_tablet_tool_v2::Event::ProximityIn { .. } => {
                state.in_proximity = true;
            }
            zwp_tablet_tool_v2::Event::ProximityOut => {
                if state.down {
                    state.down = false;
                    state.released = true;
                }
                state.in_proximity = false;
                state.stylus_btn = false;
                state.stylus2_btn = false;
                state.stylus_btn_saw_down = false;
                state.pressure = None;
            }
            zwp_tablet_tool_v2::Event::Down { .. } => {
                state.down = true;
                state.pressed = true;
                if state.stylus_btn {
                    state.stylus_btn_saw_down = true;
                }
            }
            zwp_tablet_tool_v2::Event::Up => {
                state.down = false;
                state.released = true;
            }
            zwp_tablet_tool_v2::Event::Motion { x, y } => {
                let p = tool_pos(x, y);
                state.pos = Some(p);
                if state.down {
                    state.samples.push(p);
                }
            }
            zwp_tablet_tool_v2::Event::Pressure { pressure } => {
                let p = (pressure as f32 / 65535.0).clamp(0.0, 1.0);
                if p > 0.02 {
                    state.pressure = Some(p);
                }
            }
            zwp_tablet_tool_v2::Event::Button {
                button, state: st, ..
            } => {
                let pressed = matches!(st, WEnum::Value(zwp_tablet_tool_v2::ButtonState::Pressed));
                if button == BTN_STYLUS {
                    if pressed {
                        state.stylus_btn = true;
                        state.stylus_btn_saw_down = state.down;
                    } else {
                        if state.stylus_btn
                            && !state.stylus_btn_saw_down
                            && !state.down
                            && state.in_proximity
                            && !state.eraser_tool
                        {
                            state.air_toggle = true;
                        }
                        state.stylus_btn = false;
                        state.stylus_btn_saw_down = false;
                    }
                } else if button == BTN_STYLUS2 {
                    state.stylus2_btn = pressed;
                }
            }
            zwp_tablet_tool_v2::Event::Removed => {
                if state.down {
                    state.down = false;
                    state.released = true;
                }
                state.in_proximity = false;
                tool.destroy();
            }
            _ => {}
        }
    }
}

macro_rules! noop_dispatch {
    ($iface:ty, $ev:path) => {
        impl Dispatch<$iface, ()> for TabletState {
            fn event(
                _: &mut Self,
                _: &$iface,
                event: $ev,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                let _ = event;
            }
        }
    };
}

impl Dispatch<ZwpTabletPadV2, ()> for TabletState {
    fn event(
        _: &mut Self,
        pad: &ZwpTabletPadV2,
        event: zwp_tablet_pad_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, zwp_tablet_pad_v2::Event::Removed) {
            pad.destroy();
        }
    }

    event_created_child!(TabletState, ZwpTabletPadV2, [
        zwp_tablet_pad_v2::EVT_GROUP_OPCODE => (ZwpTabletPadGroupV2, ()),
    ]);
}

impl Dispatch<ZwpTabletPadGroupV2, ()> for TabletState {
    fn event(
        _: &mut Self,
        _: &ZwpTabletPadGroupV2,
        event: zwp_tablet_pad_group_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let _ = event;
    }

    event_created_child!(TabletState, ZwpTabletPadGroupV2, [
        zwp_tablet_pad_group_v2::EVT_RING_OPCODE => (ZwpTabletPadRingV2, ()),
        zwp_tablet_pad_group_v2::EVT_STRIP_OPCODE => (ZwpTabletPadStripV2, ()),
        zwp_tablet_pad_group_v2::EVT_DIAL_OPCODE => (ZwpTabletPadDialV2, ()),
    ]);
}

noop_dispatch!(ZwpTabletPadRingV2, zwp_tablet_pad_ring_v2::Event);
noop_dispatch!(ZwpTabletPadStripV2, zwp_tablet_pad_strip_v2::Event);
noop_dispatch!(ZwpTabletPadDialV2, zwp_tablet_pad_dial_v2::Event);

#[cfg(test)]
mod tests {
    use super::{
        pinch_frame_zoom, PINCH_ZOOM_ARM, PINCH_ZOOM_FRAME_MAX, PINCH_ZOOM_FRAME_MIN,
        PINCH_ZOOM_GAIN,
    };

    #[test]
    fn pinch_zoom_ignores_scale_inside_deadzone() {
        let mut armed = false;
        let mut last = 1.0;
        assert!(
            (pinch_frame_zoom(&mut armed, &mut last, 1.001, PINCH_ZOOM_ARM) - 1.0).abs() < 1e-6
        );
        assert!(!armed);
        assert!(
            (pinch_frame_zoom(&mut armed, &mut last, 1.002, PINCH_ZOOM_ARM) - 1.0).abs() < 1e-6
        );
        assert!(!armed);
    }

    #[test]
    fn pinch_zoom_arms_with_soft_begin_then_tracks_smoothly() {
        let mut armed = false;
        let mut last = 1.0;
        let arm_z = pinch_frame_zoom(&mut armed, &mut last, 1.03, PINCH_ZOOM_ARM);
        assert!(armed);
        // Soft ease from begin: 1 + (1.03 - 1) * GAIN, clamped.
        let expect =
            (1.0 + 0.03 * PINCH_ZOOM_GAIN).clamp(PINCH_ZOOM_FRAME_MIN, PINCH_ZOOM_FRAME_MAX);
        assert!((arm_z - expect).abs() < 1e-5);
        assert!((last - 1.03).abs() < 1e-6);
        let z = pinch_frame_zoom(&mut armed, &mut last, 1.133, PINCH_ZOOM_ARM);
        // raw 1.133/1.03 ≈ 1.1 → 1 + 0.1 * GAIN, may hit the per-frame cap.
        let expect =
            (1.0 + 0.1 * PINCH_ZOOM_GAIN).clamp(PINCH_ZOOM_FRAME_MIN, PINCH_ZOOM_FRAME_MAX);
        assert!((z - expect).abs() < 1e-4);
        assert!((last - 1.133).abs() < 1e-6);
    }

    #[test]
    fn pinch_zoom_clamps_a_batched_jump() {
        let mut armed = true;
        let mut last = 1.0;
        let z = pinch_frame_zoom(&mut armed, &mut last, 1.5, PINCH_ZOOM_ARM);
        assert!((z - PINCH_ZOOM_FRAME_MAX).abs() < 1e-6);
    }
}
