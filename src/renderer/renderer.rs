use crate::camera::Camera;
use crate::planar_camera::PlanarCamera;

/// Trait implemented by custom renderer.
pub trait Renderer {
    /// Perform a rendering pass.
    fn render(&mut self, pass: usize, camera: &mut dyn Camera);
}

/// Trait implemented by customer planer renderers.
pub trait PlanarRenderer {
    /// Perform a rendering pass.
    fn render(&mut self, camera: &mut dyn PlanarCamera);
}
