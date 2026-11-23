//! STEP (.step / .stp) import: the solid-model format Fusion, SolidWorks, Onshape and FreeCAD save. The file's
//! faces (planes, cylinders, cones, spheres, tori, splines) are read with truck-stepio and turned into triangles
//! with truck-meshalgo (both Apache-2.0, unmodified), then the model goes the way an STL does: a 3D model on the
//! board to carve, with Outline as Vectors for cutting a flat part out.
//! A damaged or unusual file can stop the reader, so on desktop the read runs in a copy of this program
//! (`--step-to-stl`, like the DWG read); the web build reads in place.

use gcarve_core::relief::{Pt3, Triangle};
use gcarve_ui::i18n;
use truck_meshalgo::prelude::*;
use truck_stepio::r#in::Table;

/// ISO 10303-21 text, whatever the file is called
pub fn is_step(name: &str, bytes: &[u8]) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".step") || n.ends_with(".stp") || bytes.trim_ascii_start().starts_with(b"ISO-10303-21")
}

/// The model's triangles (the file's own units: the model is fitted to the board anyway)
pub fn load(bytes: &[u8]) -> Result<Vec<Triangle>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some((ok, out)) = crate::files::run_helper("--step-to-stl", bytes) {
        return if ok {
            gcarve_core::relief::load_stl(&out)
        } else if out.is_empty() {
            Err(i18n::t("This STEP file could not be read. Export it again from the CAD program (STEP AP214 or AP203), or save the part as STL.").into())
        } else {
            Err(String::from_utf8_lossy(&out).into_owned()) // the helper's own message
        };
    }
    load_inproc(bytes)
}

/// The in-process read (the helper process and the web build use it)
pub(crate) fn load_inproc(bytes: &[u8]) -> Result<Vec<Triangle>, String> {
    let text = String::from_utf8_lossy(bytes);
    let unreadable = || i18n::t("This STEP file could not be read. Export it again from the CAD program (STEP AP214 or AP203), or save the part as STL.").to_string();
    let tris = std::panic::catch_unwind(|| {
        let table = Table::from_step(&text)?;
        let mut out: Vec<Triangle> = Vec::new();
        // shells in the file's own order (the table is a hash map): the same file always gives the same model
        let mut ids: Vec<&u64> = table.shell.keys().collect();
        ids.sort_unstable();
        for id in ids {
            let Ok(shell) = table.to_compressed_shell(&table.shell[id]) else { continue };
            // a coarse pass for the size, then facets a thousandth of it (curves look round, files stay small)
            let size = shell.robust_triangulation(0.05).to_polygon().bounding_box().diameter();
            if !(size.is_finite() && size > 0.0) {
                continue;
            }
            let mesh = shell.robust_triangulation(size * 0.001).to_polygon();
            let p = mesh.positions();
            let at = |i: usize| Pt3 { x: p[i].x, y: p[i].y, z: p[i].z };
            out.extend(mesh.faces().triangle_iter().map(|t| Triangle { a: at(t[0].pos), b: at(t[1].pos), c: at(t[2].pos) }));
        }
        Some(out)
    })
    .map_err(|_| unreadable())?
    .ok_or_else(unreadable)?;
    if tris.is_empty() {
        return Err(i18n::t("No solid or surface model was found in this STEP file (a drawing or an empty assembly?). Export the part itself as STEP, or save it as STL.").into());
    }
    Ok(lay_flat(tris))
}

/// CAD programs don't agree on which way is up (Fusion's default is Y), and a carving wants the model's thin side as
/// its depth: when another side is clearly thinner than Z, the model is turned so that side points up (a quarter
/// turn, never mirrored).
fn lay_flat(mut tris: Vec<Triangle>) -> Vec<Triangle> {
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in tris.iter().flat_map(|t| [t.a, t.b, t.c]) {
        for (k, v) in [p.x, p.y, p.z].into_iter().enumerate() {
            (lo[k], hi[k]) = (lo[k].min(v), hi[k].max(v));
        }
    }
    let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let thin = if ext[0] < ext[1] { 0 } else { 1 };
    if ext[thin] >= 0.6 * ext[2] {
        return tris;
    }
    let turn = |p: Pt3| if thin == 1 { Pt3 { x: p.x, y: -p.z, z: p.y } } else { Pt3 { x: p.y, y: p.z, z: p.x } };
    for t in &mut tris {
        (t.a, t.b, t.c) = (turn(t.a), turn(t.b), turn(t.c));
    }
    tris
}

/// Binary STL of the triangles: what the helper process hands back
pub(crate) fn to_stl(tris: &[Triangle]) -> Vec<u8> {
    let mut out = vec![0u8; 80];
    out.extend((tris.len() as u32).to_le_bytes());
    for t in tris {
        out.extend([0u8; 12]);
        for p in [t.a, t.b, t.c] {
            for v in [p.x, p.y, p.z] {
                out.extend((v as f32).to_le_bytes());
            }
        }
        out.extend([0u8; 2]);
    }
    out
}

#[cfg(test)]
mod tests {
    /// a 30 × 20 × 10 block written as STEP (truck's own writer) comes back as a closed box of that size; text that
    /// is not a model gives a plain message; real files: GCARVE_STEP=part.step
    #[test]
    fn a_step_solid_becomes_triangles_of_the_right_size() {
        use truck_modeling::*;
        let v = builder::vertex(Point3::new(0.0, 0.0, 0.0));
        let block = builder::tsweep(&builder::tsweep(&builder::tsweep(&v, Vector3::new(30.0, 0.0, 0.0)), Vector3::new(0.0, 20.0, 0.0)), Vector3::new(0.0, 0.0, 10.0));
        let step = truck_stepio::out::CompleteStepDisplay::new(truck_stepio::out::StepModel::from(&block.compress()), Default::default()).to_string();
        assert!(super::is_step("part.STP", b"") && super::is_step("x.bin", step.as_bytes()) && !super::is_step("a.stl", b"solid"));
        let tris = super::load(step.as_bytes()).expect("reads");
        assert!(tris.len() >= 12, "{} triangles", tris.len());
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for p in tris.iter().flat_map(|t| [t.a, t.b, t.c]) {
            for (k, v) in [p.x, p.y, p.z].into_iter().enumerate() {
                (lo[k], hi[k]) = (lo[k].min(v), hi[k].max(v));
            }
        }
        for (k, want) in [30.0, 20.0, 10.0].into_iter().enumerate() {
            assert!((hi[k] - lo[k] - want).abs() < 1e-6, "axis {k}: {} to {}", lo[k], hi[k]);
        }
        // the hand-back format reads as the same model
        let back = gcarve_core::relief::load_stl(&super::to_stl(&tris)).unwrap();
        assert_eq!(back.len(), tris.len());
        assert!(super::load(b"ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n").is_err());
        assert!(super::load(b"not a step file at all").is_err());
        // a plate modelled standing up (Y up, as Fusion does) is laid flat: its thin side becomes the depth
        let stood = |t: &gcarve_core::relief::Triangle| [t.a, t.b, t.c].map(|p| gcarve_core::relief::Pt3 { x: p.x, y: p.z, z: p.y * 30.0 });
        let up: Vec<_> = tris.iter().map(|t| { let [a, b, c] = stood(t); gcarve_core::relief::Triangle { a, b, c } }).collect();
        let flat = super::lay_flat(up);
        let zs: Vec<f64> = flat.iter().flat_map(|t| [t.a.z, t.b.z, t.c.z]).collect();
        assert!((zs.iter().copied().fold(f64::MIN, f64::max) - zs.iter().copied().fold(f64::MAX, f64::min) - 10.0).abs() < 1e-6, "the 10 thick side is the depth");
        if let Ok(path) = std::env::var("GCARVE_STEP") {
            let t = super::load(&std::fs::read(&path).unwrap()).unwrap();
            println!("{path}: {} triangles", t.len());
        }
    }
}
