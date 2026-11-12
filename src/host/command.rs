use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ShareRatio {
    pub numerator: f32,
    pub denominator: f32,
}

impl ShareRatio {
    pub fn new(numerator: f32, denominator: f32) -> Option<Self> {
        if numerator > 0.0 && denominator > 0.0 {
            Some(Self {
                numerator,
                denominator,
            })
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    Below,
    Right,
}
