// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Inject real compositor-level touchpad scrolling for smoothwheel
//! verification. Creates a wlr virtual pointer, moves it over the
//! focused window, and streams finger-sourced axis events — the same
//! event class a physical two-finger touchpad swipe produces — so the
//! whole native pipeline (compositor -> GTK -> WebKit -> smoothwheel
//! script) is exercised, not synthetic page-JS WheelEvents.
//!
//! Usage: cargo run --example inject_scroll -p hwatud -- <x> <y> <dy> <events> [dx]
//! Not shipped; developer tool for scroll-path verification.

use wayland_client::protocol::{wl_output, wl_pointer, wl_registry, wl_seat};
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

struct S;
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for S {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for S {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wl_output::WlOutput, ()> for S {
    fn event(
        _: &mut Self,
        _: &wl_output::WlOutput,
        _: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for S {
    fn event(
        _: &mut Self,
        _: &ZwlrVirtualPointerManagerV1,
        _: <ZwlrVirtualPointerManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<ZwlrVirtualPointerV1, ()> for S {
    fn event(
        _: &mut Self,
        _: &ZwlrVirtualPointerV1,
        _: <ZwlrVirtualPointerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn now_ms() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u32
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let x: f64 = args[0].parse().unwrap();
    let y: f64 = args[1].parse().unwrap();
    let dy: f64 = args[2].parse().unwrap();
    let n: usize = args[3].parse().unwrap();
    let dx: f64 = args.get(4).map(|v| v.parse().unwrap()).unwrap_or(0.0);

    let conn = Connection::connect_to_env().expect("wayland connect");
    let (globals, mut queue) = registry_queue_init::<S>(&conn).expect("globals");
    let qh = queue.handle();
    let manager: ZwlrVirtualPointerManagerV1 = globals
        .bind(&qh, 1..=2, ())
        .expect("virtual pointer manager");
    let seat: Option<wl_seat::WlSeat> = globals.bind(&qh, 1..=9, ()).ok();
    let output: Option<wl_output::WlOutput> = globals.bind(&qh, 1..=4, ()).ok();
    let ptr =
        manager.create_virtual_pointer_with_output::<_, S>(seat.as_ref(), output.as_ref(), &qh, ());
    conn.flush().unwrap();
    let mut s = S;
    let _ = queue.roundtrip(&mut s);

    // Park the pointer over the target so the scroll hits the window.
    ptr.motion_absolute(now_ms(), x as u32, y as u32, 1920, 1080);
    ptr.frame();
    conn.flush().unwrap();
    let _ = queue.roundtrip(&mut s);
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Finger-sourced continuous axis stream: what a two-finger
    // touchpad swipe looks like on the wire. No axis_discrete, so
    // GTK/WebKit treat it as precise deltas. Deltas vary per frame
    // like a real finger (a perfectly uniform stream would look like
    // a quantized hi-res wheel to delta-GCD detectors, which is not
    // what a touchpad produces).
    const JITTER: [f64; 8] = [1.0, 1.31, 0.83, 1.19, 0.91, 1.27, 0.77, 1.13];
    for i in 0..n {
        let t = now_ms();
        let j = JITTER[i % JITTER.len()];
        ptr.axis_source(wl_pointer::AxisSource::Finger);
        if dy != 0.0 {
            ptr.axis(t, wl_pointer::Axis::VerticalScroll, dy * j);
        }
        if dx != 0.0 {
            ptr.axis(t, wl_pointer::Axis::HorizontalScroll, dx * j);
        }
        ptr.frame();
        conn.flush().unwrap();
        let _ = queue.roundtrip(&mut s);
        std::thread::sleep(std::time::Duration::from_millis(12));
    }
    // Scroll stop, like libinput sends on finger lift.
    let t = now_ms();
    ptr.axis_source(wl_pointer::AxisSource::Finger);
    ptr.axis_stop(t, wl_pointer::Axis::VerticalScroll);
    if dx != 0.0 {
        ptr.axis_stop(t, wl_pointer::Axis::HorizontalScroll);
    }
    ptr.frame();
    conn.flush().unwrap();
    let _ = queue.roundtrip(&mut s);
    println!("injected {n} finger axis frames at {x},{y} dy={dy} dx={dx}");
}
