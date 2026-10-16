//! Layer edits (Qt app/canvas_layers.cpp): add, remove, rename, colour, move the selection, select a layer's shapes.
//! Visible / locked are view state (no undo step), like the Qt app.

use crate::canvas::Canvas;

impl Canvas {
    pub fn add_layer(&mut self) {
        self.doc.checkpoint();
        let n = self.doc.layers.len() + 1;
        self.doc.active_layer = self.doc.layer_named(&format!("Layer {n}"));
        self.mutated();
    }

    /// Its shapes go to the first layer; later layers shift down.
    pub fn remove_layer(&mut self, i: i32) {
        let doc = &mut self.doc;
        if i < 0 || i as usize >= doc.layers.len() || doc.layers.len() == 1 {
            return;
        }
        doc.checkpoint();
        doc.layers.remove(i as usize);
        for s in &mut doc.shapes {
            if s.layer == i { s.layer = 0 } else if s.layer > i { s.layer -= 1 }
        }
        if doc.active_layer >= i {
            doc.active_layer = (doc.active_layer - 1).max(0);
        }
        self.mutated();
    }

    pub fn rename_layer(&mut self, i: usize, name: &str) {
        let name = name.trim();
        if name.is_empty() || self.doc.layers.get(i).is_none_or(|l| l.name == name) {
            return;
        }
        self.doc.checkpoint();
        self.doc.layers[i].name = name.to_string();
        self.mutated();
    }

    /// `undo`: record an undo step (the first change of a colour-picker drag), else just recolour.
    pub fn set_layer_color(&mut self, i: usize, color: &str, undo: bool) {
        if self.doc.layers.get(i).is_none_or(|l| l.color == color) {
            return;
        }
        if undo {
            self.doc.checkpoint();
        }
        self.doc.layers[i].color = color.to_string();
        self.mutated();
    }

    pub fn move_selection_to_layer(&mut self, i: i32) {
        if i < 0 || i as usize >= self.doc.layers.len() || self.sel.is_empty() {
            return;
        }
        self.doc.checkpoint();
        for id in self.sel.clone() {
            if let Some(s) = self.doc.find_mut(id) {
                s.layer = i;
            }
        }
        self.mutated();
    }

    /// Double-click on a layer: select its editable shapes and make it active.
    pub fn select_layer_shapes(&mut self, i: i32) {
        if i < 0 || i as usize >= self.doc.layers.len() {
            return;
        }
        self.sel = self.doc.shapes.iter().filter(|s| s.layer == i && self.doc.shape_editable(s)).map(|s| s.id).collect();
        self.doc.active_layer = i;
        self.sel_changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcarve_core::geom::make_rect;

    #[test]
    fn layer_edits_follow_qt() {
        let mut c = Canvas::default();
        let a = c.add_shape(make_rect(0.0, 0.0, 1.0, 1.0));
        c.add_layer();
        assert_eq!((c.doc.layers.len(), c.doc.active_layer), (2, 1));
        let b = c.add_shape(make_rect(2.0, 0.0, 1.0, 1.0));
        c.sel = [a].into();
        c.move_selection_to_layer(1);
        assert_eq!(c.doc.find(a).unwrap().layer, 1);
        c.select_layer_shapes(1);
        assert_eq!(c.sel, [a, b].into());
        c.rename_layer(1, "  Cuts ");
        c.set_layer_color(1, "#ff0000", true);
        assert_eq!((c.doc.layers[1].name.as_str(), c.doc.layers[1].color.as_str()), ("Cuts", "#ff0000"));
        c.remove_layer(1);
        assert_eq!(c.doc.layers.len(), 1);
        assert!(c.doc.shapes.iter().all(|s| s.layer == 0) && c.doc.active_layer == 0);
        c.undo();
        assert_eq!(c.doc.layers[1].name, "Cuts");
    }
}
