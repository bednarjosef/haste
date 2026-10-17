use super::*;

/// A map decoration icon.
#[derive(Clone, Debug, PartialEq)]
pub struct MapIcon {
    /// Icon type.
    pub kind: i32,
    /// X (-128..127).
    pub x: i8,
    /// Z.
    pub z: i8,
    /// Direction 0..15.
    pub direction: i8,
    /// Display name.
    pub name: Option<Component>,
}
impl Wire for MapIcon {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            kind: r.var_int()?,
            x: r.i8()?,
            z: r.i8()?,
            direction: r.i8()?,
            name: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.kind);
        w.i8(self.x);
        w.i8(self.z);
        w.i8(self.direction);
        w.write(&self.name);
    }
}

/// A rectangle of map colours.
#[derive(Clone, Debug, PartialEq)]
pub struct MapPatch {
    /// Columns (> 0).
    pub columns: u8,
    /// Rows.
    pub rows: u8,
    /// X offset.
    pub x: u8,
    /// Z offset.
    pub z: u8,
    /// Colours.
    pub data: Bytes,
}

/// Map item update.
#[derive(Clone, Debug, PartialEq)]
pub struct MapItemData {
    /// Map id.
    pub map_id: i32,
    /// Scale 0..4.
    pub scale: i8,
    /// Locked.
    pub locked: bool,
    /// Icons, if updated.
    pub icons: Option<Vec<MapIcon>>,
    /// Colour patch, if any.
    pub patch: Option<MapPatch>,
}
impl Packet for MapItemData {
    const NAME: &'static str = "minecraft:map_item_data";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let map_id = r.var_int()?;
        let scale = r.i8()?;
        let locked = r.bool()?;
        let icons = r.read()?;
        let columns = r.u8()?;
        let patch = if columns > 0 {
            Some(MapPatch {
                columns,
                rows: r.u8()?,
                x: r.u8()?,
                z: r.u8()?,
                data: r.byte_array(128 * 128)?,
            })
        } else {
            None
        };
        Ok(Self {
            map_id,
            scale,
            locked,
            icons,
            patch,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.map_id);
        w.i8(self.scale);
        w.bool(self.locked);
        w.write(&self.icons);
        match &self.patch {
            Some(p) => {
                w.u8(p.columns);
                w.u8(p.rows);
                w.u8(p.x);
                w.u8(p.z);
                w.byte_array(&p.data);
            }
            None => w.u8(0),
        }
    }
}
