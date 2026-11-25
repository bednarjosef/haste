//! Positions (a packed long), angle bytes (1/256 of a turn) and
//! low-precision vectors (velocities), all from the minecraft.wiki
//! "Data types" page.

use haste_core::{BlockPos, DVec3};

use super::{DecodeResult, Reader, Wire, Writer};

impl Reader<'_> {
    /// Position (packed long).
    pub fn block_pos(&mut self) -> DecodeResult<BlockPos> {
        Ok(BlockPos::from_packed(self.i64()?))
    }

    /// Angle: 1/256 of a turn, returned in degrees in `[0, 360)`.
    pub fn angle(&mut self) -> DecodeResult<f32> {
        Ok(self.u8()? as f32 * (360.0 / 256.0))
    }

    /// LpVec3: a low-precision vector (velocities), see the Data types page.
    pub fn lp_vec3(&mut self) -> DecodeResult<DVec3> {
        let b1 = self.u8()? as u64;
        if b1 == 0 {
            return Ok(DVec3::ZERO);
        }
        let b2 = self.u8()? as u64;
        let rest = self.u32()? as u64;
        let packed = (rest << 16) | (b2 << 8) | b1;
        let mut scale = b1 & 3;
        if b1 & 4 != 0 {
            scale |= (self.var_int()? as u32 as u64) << 2;
        }
        let scale = scale as f64;
        fn unpack(v: u64) -> f64 {
            ((v & 32767) as f64).min(32766.0) * 2.0 / 32766.0 - 1.0
        }
        Ok(DVec3::new(
            unpack(packed >> 3) * scale,
            unpack(packed >> 18) * scale,
            unpack(packed >> 33) * scale,
        ))
    }
}

impl Writer {
    /// Position.
    pub fn block_pos(&mut self, p: &BlockPos) {
        self.i64(p.to_packed());
    }
    /// Angle from degrees (wraps).
    pub fn angle(&mut self, deg: f32) {
        self.u8((deg * 256.0 / 360.0).floor() as i32 as u8);
    }
    /// LpVec3.
    pub fn lp_vec3(&mut self, v: DVec3) {
        let max = v.x.abs().max(v.y.abs()).max(v.z.abs());
        if max.is_nan() || max < 1.0 / 32766.0 {
            self.u8(0);
            return;
        }
        let scale = max.ceil() as u64;
        let cont = (scale & 3) != scale;
        let packed_scale = if cont { (scale & 3) | 4 } else { scale };
        fn pack(v: f64) -> u64 {
            ((v * 0.5 + 0.5) * 32766.0).round() as u64
        }
        let s = scale as f64;
        let packed =
            (pack(v.z / s) << 33) | (pack(v.y / s) << 18) | (pack(v.x / s) << 3) | packed_scale;
        self.u8(packed as u8);
        self.u8((packed >> 8) as u8);
        self.u32((packed >> 16) as u32);
        if cont {
            self.var_int((scale >> 2) as i32);
        }
    }
}

impl Wire for BlockPos {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        r.block_pos()
    }
    fn write(&self, w: &mut Writer) {
        w.block_pos(self)
    }
}
/// Three doubles.
impl Wire for DVec3 {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(DVec3::new(r.f64()?, r.f64()?, r.f64()?))
    }
    fn write(&self, w: &mut Writer) {
        w.f64(self.x);
        w.f64(self.y);
        w.f64(self.z);
    }
}
