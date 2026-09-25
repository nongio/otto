//! Smithay multi-GPU api for the Vulkan renderer.
//!
//! [`GbmVulkanBackend`] plays the part `GbmGlesBackend` plays for GL: one
//! [`SkiaVkRenderer`] per DRM node, on the Vulkan physical device whose
//! render or primary node matches, next to a GBM allocator on the node's
//! GBM device. Renderers are created when the node is added, so a device
//! Vulkan cannot drive is reported there instead of on first use.

// Rust guideline compliant 2026-02-21

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    fmt,
};

use smithay::backend::{
    allocator::{
        dmabuf::{AnyError, Dmabuf, DmabufAllocator},
        gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        Allocator,
    },
    drm::{DrmDeviceFd, DrmNode},
    renderer::{
        multigpu::{ApiDevice, Error as MultiError, GraphicsApi},
        RendererSuper,
    },
    vulkan::{instance::InstanceError, version::Version, Instance, PhysicalDevice},
    SwapBuffersError,
};

use crate::renderer::vulkan::{SkiaVkError, SkiaVkRenderer};

/// Errors of the [`GbmVulkanBackend`].
#[derive(Debug, thiserror::Error)]
pub enum GbmVulkanError {
    /// The Vulkan instance could not be created.
    #[error("creating the Vulkan instance failed: {0}")]
    Instance(#[from] InstanceError),
    /// Listing the physical devices failed.
    #[error("enumerating Vulkan devices failed: {0}")]
    Enumerate(#[from] smithay::reexports::ash::vk::Result),
    /// No Vulkan device drives the node.
    #[error("no Vulkan device drives {0}")]
    NoDevice(DrmNode),
    /// The renderer could not be created on the device.
    #[error("Vulkan renderer on {node}: {source}")]
    Renderer {
        /// The node the renderer was for.
        node: DrmNode,
        /// What went wrong.
        #[source]
        source: SkiaVkError,
    },
}

impl From<GbmVulkanError> for SwapBuffersError {
    fn from(err: GbmVulkanError) -> Self {
        SwapBuffersError::ContextLost(Box::new(err))
    }
}

/// A [`GraphicsApi`] of GBM devices rendered with Skia on Vulkan.
pub struct GbmVulkanBackend {
    instance: Option<Instance>,
    nodes: HashSet<DrmNode>,
    /// Devices created by `add_node`, waiting for the next enumeration.
    pending: RefCell<Vec<GbmVulkanDevice>>,
    /// A node was removed and its device is still in the manager's list.
    removed: Cell<bool>,
    allocator_flags: GbmBufferFlags,
}

impl Default for GbmVulkanBackend {
    fn default() -> Self {
        Self {
            instance: None,
            nodes: HashSet::new(),
            pending: RefCell::new(Vec::new()),
            removed: Cell::new(false),
            allocator_flags: GbmBufferFlags::RENDERING,
        }
    }
}

impl fmt::Debug for GbmVulkanBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GbmVulkanBackend")
            .field("nodes", &self.nodes)
            .finish_non_exhaustive()
    }
}

impl GbmVulkanBackend {
    /// Adds the GBM device of `node`, creating its renderer.
    ///
    /// # Errors
    ///
    /// Fails when there is no Vulkan instance, no Vulkan device for the node,
    /// or the renderer cannot be created on it.
    pub fn add_node(
        &mut self,
        node: DrmNode,
        gbm: GbmDevice<DrmDeviceFd>,
    ) -> Result<(), GbmVulkanError> {
        if self.nodes.contains(&node) {
            return Ok(());
        }
        let instance = match &self.instance {
            Some(instance) => instance.clone(),
            None => {
                let instance = Instance::new(Version::VERSION_1_3, None)?;
                self.instance = Some(instance.clone());
                instance
            }
        };
        let matches = |candidate: Option<DrmNode>| {
            candidate.is_some_and(|candidate| candidate.dev_id() == node.dev_id())
        };
        let phd = PhysicalDevice::enumerate(&instance)?
            .find(|phd| {
                matches(phd.render_node().ok().flatten())
                    || matches(phd.primary_node().ok().flatten())
            })
            .ok_or(GbmVulkanError::NoDevice(node))?;
        let renderer = SkiaVkRenderer::new(&phd)
            .map_err(|source| GbmVulkanError::Renderer { node, source })?;

        self.pending.get_mut().push(GbmVulkanDevice {
            node,
            renderer,
            allocator: Box::new(DmabufAllocator(GbmAllocator::new(
                gbm,
                self.allocator_flags,
            ))),
        });
        self.nodes.insert(node);
        Ok(())
    }

    /// Removes `node` and drops its renderer on the next enumeration.
    pub fn remove_node(&mut self, node: &DrmNode) {
        if self.nodes.remove(node) {
            self.removed.set(true);
        }
        self.pending.get_mut().retain(|device| device.node != *node);
    }
}

impl GraphicsApi for GbmVulkanBackend {
    type Device = GbmVulkanDevice;
    type Error = GbmVulkanError;

    fn enumerate(&self, list: &mut Vec<Self::Device>) -> Result<(), Self::Error> {
        list.retain(|device| self.nodes.contains(&device.node));
        self.removed.set(false);
        list.append(&mut self.pending.borrow_mut());
        Ok(())
    }

    fn needs_enumeration(&self) -> bool {
        self.removed.get() || !self.pending.borrow().is_empty()
    }

    fn identifier() -> &'static str {
        "gbm_vulkan"
    }
}

#[cfg(feature = "egl")]
impl smithay::backend::renderer::multigpu::EglImportUnsupported for GbmVulkanBackend {}

impl<T: GraphicsApi> From<SkiaVkError> for MultiError<GbmVulkanBackend, T>
where
    T::Error: 'static,
    <<T::Device as ApiDevice>::Renderer as RendererSuper>::Error: 'static,
{
    fn from(err: SkiaVkError) -> Self {
        MultiError::Render(err)
    }
}

/// A DRM node's renderer and allocator.
pub struct GbmVulkanDevice {
    node: DrmNode,
    renderer: SkiaVkRenderer,
    allocator: Box<dyn Allocator<Buffer = Dmabuf, Error = AnyError>>,
}

impl fmt::Debug for GbmVulkanDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GbmVulkanDevice")
            .field("node", &self.node)
            .field("renderer", &self.renderer)
            .finish_non_exhaustive()
    }
}

impl ApiDevice for GbmVulkanDevice {
    type Renderer = SkiaVkRenderer;

    fn renderer(&self) -> &Self::Renderer {
        &self.renderer
    }

    fn renderer_mut(&mut self) -> &mut Self::Renderer {
        &mut self.renderer
    }

    fn allocator(&mut self) -> &mut dyn Allocator<Buffer = Dmabuf, Error = AnyError> {
        self.allocator.as_mut()
    }

    fn node(&self) -> &DrmNode {
        &self.node
    }

    fn can_do_cross_device_imports(&self) -> bool {
        true
    }
}
