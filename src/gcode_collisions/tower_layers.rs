//! The prime tower layer by layer, read from G-code: the first layer holds the column and its
//! brim, every later layer the column or a chamfer ring around it. The packer reserves the
//! union of these measured layers (`colored_3mf_packer::tower`).

use super::reader::{Extrusion, ExtrusionSink, read};
use crate::geometry::Rect;
use serde::{Deserialize, Serialize};

/// One printed layer of the tower.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct TowerLayer {
    /// 1-based layer; 1 is the first printed layer.
    pub layer: usize,
    /// The layer occupies `(z_bottom, z]`.
    pub z_bottom: f64,
    pub z: f64,
    /// Material extent of every tower line of the layer, half the line width included, arcs
    /// at their exact extent, plate coordinates.
    pub extent: Rect,
}

#[derive(Default)]
struct Layers(Vec<TowerLayer>);

impl ExtrusionSink for Layers {
    fn extrusion(&mut self, extrusion: &Extrusion) {
        if !extrusion.role.is_tower() || extrusion.layer == 0 {
            return;
        }
        let extent = extrusion.extent.grown(extrusion.width * 0.5);
        match self.0.last_mut() {
            Some(last) if last.layer == extrusion.layer => last.extent = last.extent.union(extent),
            _ => self.0.push(TowerLayer {
                layer: extrusion.layer,
                z_bottom: extrusion.z_bottom,
                z: extrusion.z,
                extent,
            }),
        }
    }
}

/// The tower's layers in print order; empty without a tower.
pub fn tower_layers(gcode: &[u8]) -> Vec<TowerLayer> {
    let mut layers = Layers::default();
    read(gcode, &mut layers);
    layers.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOWER: &[u8] = b"; CHANGE_LAYER\n; Z_HEIGHT: 0.2\n; LINE_WIDTH: 0.4\n; WIPE_TOWER_START\nG1 X10 Y10\nG1 X20 Y10 E1\nG1 X20 Y20 E1\n; WIPE_TOWER_END\n; CHANGE_LAYER\n; Z_HEIGHT: 0.48\n; WIPE_TOWER_START\nG1 X12 Y12\nG1 X18 Y12 E1\n; WIPE_START\nG1 X90 Y90 E1\n; WIPE_END\n; WIPE_TOWER_END\n";

    #[test]
    fn layers_hold_the_tower_extent_with_the_line_width_and_skip_the_nozzle_wipe() {
        let layers = tower_layers(TOWER);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].extent, Rect::new(9.8, 9.8, 20.2, 20.2));
        assert_eq!((layers[1].z_bottom, layers[1].z), (0.2, 0.48));
        assert_eq!(layers[1].extent, Rect::new(11.8, 11.8, 18.2, 12.2));
    }
}
