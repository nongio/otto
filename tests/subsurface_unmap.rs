//! A subsurface leaves the screen when its `wl_subsurface` is destroyed (headless).
//!
//! Chrome draws its omnibox dropdown into a desync subsurface of the window
//! and keeps the `wl_surface` across shows. It hides the dropdown with
//! `wl_surface.attach(NULL)`, `wl_surface.commit` and `wl_subsurface.destroy`.
//! When the commit before that carried a fenced dmabuf, the NULL commit waits
//! behind the fence while the destroy is processed at once, so the compositor
//! sees the NULL commit on a surface that already has no parent. Destroying a
//! `wl_subsurface` unmaps the surface by itself too, NULL commit or not.
//!
//! The headless backend imports no dmabufs, so the fence is a blocker the
//! test adds to the surface's next commit and signals by hand. "On screen" is
//! judged from the scene, as there is no renderer: a layer of the
//! subsurface's `wl_surface` with every ancestor visible and a non-empty
//! size.

// Rust guideline compliant 2026-02-21

#[cfg(feature = "headless")]
mod subsurface_unmap_tests {
    use layers::engine::scene::SceneNodeSnapshot;
    use otto::{
        headless::{HeadlessConfig, HeadlessData, HeadlessHandle},
        state::Otto,
    };
    use otto_kit::testing::{ShmBuffer, TestClient, TestToplevel};
    use serial_test::serial;
    use smithay::{
        reexports::wayland_server::{protocol::wl_surface::WlSurface as ServerSurface, Resource},
        wayland::compositor::{add_blocker, Blocker, BlockerState, CompositorHandler},
    };
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
        time::Duration,
    };
    use wayland_client::{
        protocol::{wl_subsurface::WlSubsurface, wl_surface::WlSurface},
        Proxy,
    };

    const TITLE: &str = "subsurface-unmap-window";
    /// The dropdown's place and size in the window, in logical points.
    const SUB_X: i32 = 40;
    const SUB_Y: i32 = 30;
    const SUB_W: u32 = 160;
    const SUB_H: u32 = 90;
    /// Upper bound on the engine frames `settle` runs.
    const SETTLE_FRAMES: usize = 300;

    /// A buffer fence the test signals by hand.
    struct Fence(Arc<AtomicBool>);

    impl Blocker for Fence {
        fn state(&self) -> BlockerState {
            if self.0.load(Ordering::Acquire) {
                BlockerState::Released
            } else {
                BlockerState::Pending
            }
        }
    }

    /// A mapped, focused window with a desync subsurface showing a buffer.
    struct Scene {
        handle: HeadlessHandle,
        client: TestClient,
        toplevel: Arc<Mutex<TestToplevel>>,
        content: WlSurface,
        subsurface: WlSubsurface,
        _buffer: ShmBuffer,
    }

    fn start() -> Scene {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");
        let toplevel = client.create_toplevel(TITLE, 640, 480);
        handle.wait(Duration::from_millis(100));
        client.roundtrip().expect("roundtrip");
        handle.settle(SETTLE_FRAMES);
        handle.focus_window(TITLE);
        handle.settle(SETTLE_FRAMES);

        let parent = toplevel.lock().unwrap().surface.clone();
        let (content, subsurface, buffer) =
            client.create_subsurface(&parent, SUB_X, SUB_Y, SUB_W, SUB_H);
        subsurface.set_desync();
        client.roundtrip().expect("roundtrip");
        handle.settle(SETTLE_FRAMES);

        Scene {
            handle,
            client,
            toplevel,
            content,
            subsurface,
            _buffer: buffer,
        }
    }

    /// The compositor's side of the client surface with `protocol_id`.
    fn server_surface(state: &Otto<HeadlessData>, protocol_id: u32) -> ServerSurface {
        let id = state
            .surface_layers
            .keys()
            .find(|id| id.protocol_id() == protocol_id && id.interface().name == "wl_surface")
            .expect("the subsurface has a layer")
            .clone();
        ServerSurface::from_id(&state.display_handle, id).expect("the surface is alive")
    }

    impl Scene {
        /// Let the compositor take everything sent so far and draw it.
        fn sync(&mut self) {
            self.client.roundtrip().expect("roundtrip");
            self.handle.settle(SETTLE_FRAMES);
        }

        /// The window draws a new frame of its own.
        fn commit_parent(&mut self) {
            let toplevel = self.toplevel.lock().unwrap();
            let surface = &toplevel.surface;
            if let Some(buffer) = toplevel.buffer.as_ref() {
                surface.attach(Some(buffer), 0, 0);
            }
            surface.damage(0, 0, i32::MAX, i32::MAX);
            surface.commit();
        }

        /// Hold `content`'s next commit behind a fence, the way a dmabuf
        /// with an acquire point holds it. Returns the fence's signal.
        fn fence_next_commit(&self) -> Arc<AtomicBool> {
            let signalled = Arc::new(AtomicBool::new(false));
            let fence = Fence(signalled.clone());
            let protocol_id = self.content.id().protocol_id();
            self.handle.with_state(move |state| {
                add_blocker(&server_surface(state, protocol_id), fence);
            });
            signalled
        }

        /// Signal the fence: the held commit is applied and reaches the
        /// compositor's commit handler.
        fn signal(&self, signalled: Arc<AtomicBool>) {
            let protocol_id = self.content.id().protocol_id();
            self.handle.with_state(move |state| {
                signalled.store(true, Ordering::Release);
                let surface = server_surface(state, protocol_id);
                let client = surface.client().expect("the client is connected");
                let dh = state.display_handle.clone();
                state
                    .client_compositor_state(&client)
                    .blocker_cleared(state, &dh);
            });
        }

        /// Chrome's hide with the NULL commit held behind a fence: the
        /// commit, the `wl_subsurface.destroy` that follows it, and only
        /// then the fence.
        fn hide_behind_fence(&mut self) {
            let fence = self.fence_next_commit();
            self.content.attach(None, 0, 0);
            self.content.commit();
            self.subsurface.destroy();
            self.sync();
            self.signal(fence);
            self.handle.settle(SETTLE_FRAMES);
        }

        /// Hang `content` off the window again and give it a buffer.
        fn reattach(&mut self) -> (WlSubsurface, ShmBuffer) {
            let subcompositor = self
                .client
                .state
                .wl_subcompositor
                .clone()
                .expect("wl_subcompositor");
            let parent = self.toplevel.lock().unwrap().surface.clone();
            let subsurface =
                subcompositor.get_subsurface(&self.content, &parent, &self.client.qh, ());
            subsurface.set_position(SUB_X, SUB_Y);
            subsurface.set_desync();
            let buffer = ShmBuffer::new(
                self.client.state.wl_shm.as_ref().expect("wl_shm"),
                &self.client.qh,
                SUB_W,
                SUB_H,
            );
            self.content.attach(Some(buffer.buffer()), 0, 0);
            self.content.damage(0, 0, SUB_W as i32, SUB_H as i32);
            self.content.commit();
            self.commit_parent();
            (subsurface, buffer)
        }

        /// Layers of `content` drawn in the scene, as `key @ (x, y, w, h)`
        /// in physical pixels.
        fn drawn(&self) -> Vec<String> {
            // lay-rs keys are `surface_{ObjectId:?}`, and a server `ObjectId`
            // prints as `ObjectId(wl_surface@<id>)`. The test is the only
            // client, so the protocol id is unambiguous.
            let fragment = format!("(wl_surface@{})", self.content.id().protocol_id());
            let snapshot = self.handle.scene_snapshot();
            let mut drawn = Vec::new();
            for node in &snapshot.nodes {
                collect(node, &fragment, true, &mut drawn);
            }
            drawn
        }

        fn assert_drawn(&self, scenario: &str) -> Vec<String> {
            let drawn = self.drawn();
            assert_eq!(
                drawn.len(),
                1,
                "[{scenario}] the subsurface is drawn once: {drawn:#?}"
            );
            drawn
        }

        fn assert_not_drawn(&self, scenario: &str) {
            let drawn = self.drawn();
            assert!(
                drawn.is_empty(),
                "[{scenario}] a layer of the unmapped subsurface is still drawn: {drawn:#?}"
            );
        }
    }

    /// Walk the tree, recording drawn layers whose key contains `fragment`;
    /// `visible` is whether every ancestor of `node` is drawn.
    fn collect(node: &SceneNodeSnapshot, fragment: &str, visible: bool, drawn: &mut Vec<String>) {
        let visible = visible && !node.hidden && node.opacity > 0.0;
        let b = &node.global_bounds;
        if visible
            && node.key.starts_with("surface_")
            && node.key.contains(fragment)
            && b.width > 0.0
            && b.height > 0.0
        {
            drawn.push(format!(
                "{} @ ({}, {}, {}, {})",
                node.key, b.x, b.y, b.width, b.height
            ));
        }
        for child in &node.children {
            collect(child, fragment, visible, drawn);
        }
    }

    /// (a) The NULL commit waits behind a fence while the destroy after it
    /// goes through: the commit handler sees a surface with no parent.
    #[test]
    #[serial]
    fn a_null_commit_held_past_the_subsurface_destroy_unmaps_it() {
        let mut scene = start();
        scene.assert_drawn("fenced null commit / before");

        scene.hide_behind_fence();
        scene.assert_not_drawn("fenced null commit / before the window commits");

        scene.commit_parent();
        scene.sync();
        scene.assert_not_drawn("fenced null commit");

        scene.handle.stop();
    }

    /// (b) The client destroys the `wl_subsurface` and commits nothing on
    /// the surface: the window's next frame no longer shows it.
    #[test]
    #[serial]
    fn destroying_the_subsurface_alone_unmaps_it() {
        let mut scene = start();
        scene.assert_drawn("destroy only / before");

        scene.subsurface.destroy();
        scene.commit_parent();
        scene.sync();
        scene.assert_not_drawn("destroy only");

        scene.handle.stop();
    }

    /// (c) After (a), the same `wl_surface` becomes a subsurface again and
    /// comes back where it was.
    #[test]
    #[serial]
    fn a_reused_surface_shows_again_after_the_destroy() {
        let mut scene = start();
        let before = scene.assert_drawn("reuse / before");

        scene.hide_behind_fence();
        scene.assert_not_drawn("reuse / hidden");

        let (subsurface, buffer) = scene.reattach();
        scene.sync();
        let after = scene.assert_drawn("reuse / shown again");
        assert_eq!(
            before, after,
            "the reused subsurface is drawn where it was the first time"
        );

        drop(subsurface);
        drop(buffer);
        scene.handle.stop();
    }

    /// (d) The ordinary order: NULL commit while still a subsurface, then
    /// the destroy.
    #[test]
    #[serial]
    fn a_null_commit_before_the_subsurface_destroy_unmaps_it() {
        let mut scene = start();
        scene.assert_drawn("null commit, then destroy / before");

        scene.content.attach(None, 0, 0);
        scene.content.commit();
        scene.subsurface.destroy();
        scene.commit_parent();
        scene.sync();
        scene.assert_not_drawn("null commit, then destroy");

        scene.handle.stop();
    }
}
