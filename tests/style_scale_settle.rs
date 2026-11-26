//! A card placed and zoomed in through the surface-style protocol comes to
//! rest exactly at full size and on whole pixels.
//!
//! The launcher's card arrives the way these tests drive it: a subsurface of
//! an overlay layer surface, its style scaled just short of full size, then
//! sprung up to `1.0` in a transaction of its own while the card's size and
//! position spring in another. Anything but exactly `1.0`, or an edge
//! between two pixels, once it is still leaves the card's text resampled.

#[cfg(feature = "headless")]
mod style_scale_settle_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::protocols::{otto_surface_style_v1, otto_timing_function_v1};
    use otto_kit::surfaces::StyleFrame;
    use otto_kit::testing::{KeyboardInteractivity, Layer, TestClient};
    use serial_test::serial;
    use std::time::Duration;

    const OPEN_SCALE: f64 = 0.96;
    const SCALE_IN: f64 = 0.34;
    const BOUNCE: f64 = 0.35;
    const ANCHOR: (f64, f64) = (0.5, 0.0);
    /// Where the card sits, in points: mid-pixel at a scale of 1.75.
    const ORIGIN: (f32, f32) = (101.0, 133.0);
    const WIDTH: f32 = 620.0;

    /// Send `changes` in a transaction of `duration`, on a spring or an
    /// ease-out, the way the launcher's `animate` does.
    fn animate(
        client: &mut TestClient,
        duration: f64,
        spring: Option<f64>,
        changes: impl FnOnce(),
    ) {
        let manager = client
            .state
            .otto_surface_style_manager
            .clone()
            .expect("otto_surface_style_manager_v1 bound");
        let timing = manager.create_timing_function(&client.qh, ());
        match spring {
            Some(bounce) => timing.set_spring(bounce, 0.0),
            None => timing.set_preset(otto_timing_function_v1::Preset::EaseOutQuad),
        }
        let transaction = manager.begin_transaction(&client.qh, ());
        transaction.set_duration(duration);
        transaction.set_timing_function(&timing);
        changes();
        transaction.commit();
        let _ = client.roundtrip();
    }

    /// The card's style layer: its model scale and the scale it is drawn at.
    fn card_scale(handle: &HeadlessHandle) -> ((f32, f32), (f32, f32)) {
        handle.query(|state| {
            let style = state
                .surfaces_style
                .values()
                .flatten()
                .next()
                .expect("the card has a style");
            let model = style.layer.scale();
            let drawn = style.layer.render_layer().transform_33;
            ((model.x, model.y), (drawn.scale_x(), drawn.scale_y()))
        })
    }

    struct Card {
        client: TestClient,
        style: otto_surface_style_v1::OttoSurfaceStyleV1,
        _keep: Vec<Box<dyn std::any::Any>>,
    }

    /// Place the card `height` points tall, the way the launcher does.
    fn place(style: &otto_surface_style_v1::OttoSurfaceStyleV1, height: f32, scale: f64) {
        let frame = StyleFrame::on_pixel_grid(ORIGIN, (WIDTH, height), ANCHOR, scale);
        style.set_position(frame.position.0, frame.position.1);
        style.set_size(frame.size.0, frame.size.1);
    }

    fn open_card(handle: &HeadlessHandle, scale: f64) -> Card {
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");
        let parent = client.create_layer_surface(
            "otto-launcher",
            Layer::Overlay,
            1200,
            800,
            KeyboardInteractivity::None,
        );
        let parent_surface = parent.lock().unwrap().surface.clone();
        let (card, subsurface, buffer) =
            client.create_subsurface(&parent_surface, ORIGIN.0 as i32, ORIGIN.1 as i32, 620, 400);
        let manager = client
            .state
            .otto_surface_style_manager
            .clone()
            .expect("otto_surface_style_manager_v1 bound");
        let style = manager.get_surface_style(&card, &client.qh, ());
        style.set_anchor_point(ANCHOR.0, ANCHOR.1);
        place(&style, 400.0, scale);
        style.set_scale(OPEN_SCALE, OPEN_SCALE);
        style.set_opacity(0.0);
        card.commit();
        parent_surface.commit();
        let _ = client.roundtrip();
        handle.settle(100);
        Card {
            client,
            style,
            _keep: vec![
                Box::new(parent),
                Box::new(card),
                Box::new(subsurface),
                Box::new(buffer),
            ],
        }
    }

    fn assert_full_size(handle: &HeadlessHandle, what: &str) {
        let (model, drawn) = card_scale(handle);
        assert_eq!(
            model,
            (1.0, 1.0),
            "{what}: the card's scale settled at {model:?}"
        );
        assert_eq!(drawn, (1.0, 1.0), "{what}: the card is drawn at {drawn:?}");
    }

    /// The card's drawn edges, in output pixels.
    fn card_bounds(handle: &HeadlessHandle) -> (f32, f32, f32, f32) {
        handle.query(|state| {
            let style = state
                .surfaces_style
                .values()
                .flatten()
                .next()
                .expect("the card has a style");
            let b = style.layer.render_layer().global_transformed_bounds;
            (b.left, b.top, b.right, b.bottom)
        })
    }

    /// Drive the engine at uneven frame times, the way a real output does
    /// while clients repaint, until nothing is pending.
    fn run_uneven(handle: &HeadlessHandle) {
        let steps = [0.016_f32, 0.0083, 0.033, 0.011, 0.016, 0.05, 0.007];
        for i in 0..400 {
            handle.tick(steps[i % steps.len()]);
        }
        handle.settle(200);
    }

    #[test]
    #[serial]
    fn spring_open_settles_at_one() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle.settle(200);
        let mut card = open_card(&handle, 1.0);
        let style = card.style.clone();
        animate(&mut card.client, 0.09, None, || style.set_opacity(1.0));
        animate(&mut card.client, SCALE_IN, Some(BOUNCE), || {
            style.set_scale(1.0, 1.0)
        });
        run_uneven(&handle);
        assert_full_size(&handle, "open");
    }

    #[test]
    #[serial]
    fn interrupted_open_rests_full_size_on_whole_pixels_at_a_fractional_scale() {
        const SCALE: f64 = 1.75;
        let handle = HeadlessHandle::start(HeadlessConfig {
            scale: Some(SCALE),
            ..HeadlessConfig::default()
        });
        handle.settle(200);
        let mut card = open_card(&handle, SCALE);
        let style = card.style.clone();
        animate(&mut card.client, 0.09, None, || style.set_opacity(1.0));
        animate(&mut card.client, SCALE_IN, Some(BOUNCE), || {
            style.set_scale(1.0, 1.0)
        });
        // The card grows while it is still arriving: the agents list or the
        // log springing in under the field, a few times in a row, and the
        // same scale sent again on the way.
        for (i, height) in [431.0, 517.0, 489.0, 603.0].into_iter().enumerate() {
            for _ in 0..(3 + i) {
                handle.tick(0.016);
            }
            animate(&mut card.client, SCALE_IN, Some(BOUNCE), || {
                place(&style, height, SCALE);
            });
        }
        animate(&mut card.client, SCALE_IN, Some(BOUNCE), || {
            style.set_scale(1.0, 1.0)
        });
        handle.wait(Duration::from_millis(40));
        run_uneven(&handle);
        assert_full_size(&handle, "interrupted open");
        let edges = card_bounds(&handle);
        let (left, top, right, bottom) = edges;
        for edge in [left, top, right, bottom] {
            assert_eq!(
                edge,
                edge.round(),
                "the card rests between pixels: {edges:?}"
            );
        }
        assert_eq!(left, (ORIGIN.0 as f64 * SCALE).round() as f32, "{edges:?}");
    }
}
