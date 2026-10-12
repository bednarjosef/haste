pub use glam::{DVec3, IVec3, Mat4, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Axis-aligned box in world space, f64 like vanilla's AABB.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Aabb {
    pub min: DVec3,
    pub max: DVec3,
}

impl Aabb {
    pub fn new(min: DVec3, max: DVec3) -> Self { Self { min: min.min(max), max: min.max(max) } }
    pub fn from_min_size(min: DVec3, size: DVec3) -> Self { Self { min, max: min + size } }
    /// Vanilla entity box: feet at `pos`, centred horizontally.
    pub fn entity(pos: DVec3, width: f64, height: f64) -> Self {
        let h = width / 2.0;
        Self { min: DVec3::new(pos.x - h, pos.y, pos.z - h), max: DVec3::new(pos.x + h, pos.y + height, pos.z + h) }
    }
    pub fn offset(&self, d: DVec3) -> Self { Self { min: self.min + d, max: self.max + d } }
    pub fn expand(&self, d: DVec3) -> Self { Self { min: self.min - d, max: self.max + d } }
    pub fn intersects(&self, o: &Aabb) -> bool {
        self.min.x < o.max.x && self.max.x > o.min.x && self.min.y < o.max.y && self.max.y > o.min.y && self.min.z < o.max.z && self.max.z > o.min.z
    }
    pub fn size(&self) -> DVec3 { self.max - self.min }
    pub fn center(&self) -> DVec3 { (self.min + self.max) * 0.5 }
}
