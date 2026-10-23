//! Toolpath IR: what the CAM code produces and post-processors consume (C++ `core/job.h`). Units are inches.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MoveType {
    #[default]
    Rapid, // G0
    Linear, // G1
    ArcCw,  // G2
    ArcCcw, // G3
}

/// One move. Missing axes keep their previous value (modal). `i`/`j` = arc centre offset from the start (XY only).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Move {
    pub kind: MoveType,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z: Option<f64>,
    pub i: f64,
    pub j: f64,
}

impl Move {
    pub fn rapid(x: Option<f64>, y: Option<f64>, z: Option<f64>) -> Self {
        Move { kind: MoveType::Rapid, x, y, z, ..Default::default() }
    }
    pub fn linear(x: Option<f64>, y: Option<f64>, z: Option<f64>) -> Self {
        Move { kind: MoveType::Linear, x, y, z, ..Default::default() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toolpath {
    pub name: String,
    /// overrides the operation's tool when >= 0 (e.g. the v-carve clearance pass)
    pub tool: i32,
    pub feed_xy: f64, // inches/min
    pub feed_z: f64,
    pub moves: Vec<Move>,
    /// cutter compensation the post writes round the cutting moves: 41 = G41 (cutter left), 42 = G42, 0 = none
    pub comp: u8,
}

/// No bit override by default (`tool` -1 = the operation's bit): 0 would send the moves to the first bit's file.
impl Default for Toolpath {
    fn default() -> Self {
        Toolpath { name: String::new(), tool: -1, feed_xy: 0.0, feed_z: 0.0, moves: Vec::new(), comp: 0 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub filename: String,
    /// preformatted by the caller, e.g. "09:09 PM, Tuesday December 17 2024"
    pub timestamp: String,
    /// e.g. `48.000" x 48.000" x 0.484"`
    pub material: String,
    /// e.g. `V-Bit (90 deg 0.5")`
    pub tool: String,
    pub rpm: i32,
    pub toolpaths: Vec<Toolpath>,
    /// header text: "Material Surface" or "Table"
    pub z_zero: String,
    /// extra header comment lines (e.g. the air-cut warning)
    pub notes: Vec<String>,
    /// tool-changer slot of the bit (T number in tool-change posts); 0 = numbered in order
    pub slot: i32,
    /// rotary job: the moves' Y is the A axis in degrees, written as A by the posts
    pub a_axis: bool,
    /// rotary along the machine's Y: the posts write the X words (the length) as Y and the Y words as A
    pub a_along_y: bool,
}

impl Default for Job {
    fn default() -> Self {
        Job {
            filename: String::new(),
            timestamp: String::new(),
            material: String::new(),
            tool: String::new(),
            rpm: 18000,
            toolpaths: Vec::new(),
            z_zero: "Material Surface".into(),
            notes: Vec::new(),
            slot: 0,
            a_axis: false,
            a_along_y: false,
        }
    }
}
