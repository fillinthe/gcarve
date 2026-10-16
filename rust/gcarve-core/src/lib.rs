//! GCarve CAM engine (Rust port of the C++ `core/` + `post/`). No UI and no file system: inputs and outputs are
//! values and strings, so the same code runs on the desktop and in the browser (wasm32).
//! Every module is checked against the C++ engine's output (golden files in `reference/` and `rust/testdata/`).

pub mod arcfit;
pub mod cam;
pub mod catalog;
pub mod clipart;
pub mod document;
pub mod dxf;
pub mod expr;
pub mod feeds;
pub mod gcode;
pub mod geom;
pub mod gimport;
pub mod grbl;
pub mod job;
pub mod nest;
pub mod order;
pub mod pdf;
pub mod photo;
pub mod pipeline;
pub mod postdef;
pub mod project;
pub mod relief;
pub mod setupsheet;
pub mod sim;
pub mod svg;
pub mod template;
pub mod trace;
pub mod vecops;
pub mod wincnc;

pub use job::{Job, Move, MoveType, Toolpath};
pub use wincnc::post_wincnc;

pub use arcfit::fit_arcs;
pub use expr::eval_expr;
pub use gcode::{parse_program, GMotion, GProgram};
pub use postdef::{find_post, post_jobs, post_library, PostDef, PostOutput};
