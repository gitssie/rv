use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const WORKFLOW_VERSION: u16 = 1;
pub const MAX_INPUT_UTF16: usize = 128;

/// Native screenshot pixels and ZXTouch orientation quadrant (0..=3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub orientation: u8,
}
impl Frame {
    pub fn is_valid(self) -> bool {
        (1..=16384).contains(&self.width)
            && (1..=16384).contains(&self.height)
            && self.orientation <= 3
    }
    pub fn contains(self, point: Point) -> bool {
        self.is_valid()
            && point.x.is_finite()
            && point.y.is_finite()
            && point.x >= 0.
            && point.y >= 0.
            && point.x < f64::from(self.width)
            && point.y < f64::from(self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub fn center(self) -> Point {
        Point {
            x: self.x + self.width / 2.,
            y: self.y + self.height / 2.,
        }
    }
    pub fn is_inside(self, frame: Frame) -> bool {
        frame.contains(Point {
            x: self.x,
            y: self.y,
        }) && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.
            && self.height > 0.
            && self.x + self.width <= f64::from(frame.width)
            && self.y + self.height <= f64::from(frame.height)
    }
    pub(crate) fn is_normalized(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|n| n.is_finite())
            && self.x >= 0.
            && self.y >= 0.
            && self.width > 0.
            && self.height > 0.
            && self.x + self.width <= 1.
            && self.y + self.height <= 1.
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextMatch {
    Exact,
    Contains,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextSelector {
    pub text: String,
    pub mode: TextMatch,
    pub case_sensitive: bool,
    /// Optional region in fractions of the native screenshot; tests box centers.
    pub region: Option<Rect>,
}
impl TextSelector {
    pub(crate) fn is_valid(&self) -> bool {
        !self.text.trim().is_empty()
            && self.text.len() <= 1024
            && self.region.is_none_or(Rect::is_normalized)
    }
    pub(crate) fn matches(&self, element: &crate::TextElement, frame: Frame) -> bool {
        let center = element.bounds.center();
        if let Some(region) = self.region {
            let x = center.x / f64::from(frame.width);
            let y = center.y / f64::from(frame.height);
            if x < region.x
                || x > region.x + region.width
                || y < region.y
                || y > region.y + region.height
            {
                return false;
            }
        }
        let (candidate, expected) = if self.case_sensitive {
            (element.text.trim().to_owned(), self.text.trim().to_owned())
        } else {
            (
                element.text.trim().to_lowercase(),
                self.text.trim().to_lowercase(),
            )
        };
        match self.mode {
            TextMatch::Exact => candidate == expected,
            TextMatch::Contains => candidate.contains(&expected),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    /// Center of exactly one text box plus an offset in screen fractions.
    Text {
        selector: TextSelector,
        offset: Point,
    },
    /// A point in [0, 1) screen fractions, resolved against a fresh observation.
    Screen { point: Point },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    WaitForText {
        selector: TextSelector,
        present: bool,
    },
    Tap {
        target: Target,
    },
    /// Reference to a separately provided runtime value, never a literal secret.
    InputText {
        input: String,
    },
    Delay {
        milliseconds: u64,
    },
}
impl Action {
    pub(crate) fn is_valid(&self) -> bool {
        match self {
            Self::WaitForText { selector, .. } => selector.is_valid(),
            Self::Tap {
                target: Target::Text { selector, offset },
            } => {
                selector.is_valid()
                    && [offset.x, offset.y]
                        .iter()
                        .all(|n| n.is_finite() && n.abs() <= 1.)
            }
            Self::Tap {
                target: Target::Screen { point },
            } => [point.x, point.y]
                .iter()
                .all(|n| n.is_finite() && (0. ..1.).contains(n)),
            Self::InputText { input } => valid_id(input),
            Self::Delay { milliseconds } => *milliseconds <= 60_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub timeout_ms: u64,
    pub action: Action,
}

/// Serializable definition only. Store through RV's existing SQLite layer when
/// a business module owns it; this crate does not create a second config store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub version: u16,
    pub bundle_id: String,
    pub steps: Vec<Step>,
}
impl Workflow {
    pub fn is_valid(&self) -> bool {
        self.version == WORKFLOW_VERSION
            && valid_bundle_id(&self.bundle_id)
            && !self.steps.is_empty()
            && self.steps.len() <= 256
            && self.steps.iter().enumerate().all(|(i, step)| {
                valid_id(&step.id)
                    && self.steps[..i].iter().all(|other| other.id != step.id)
                    && (1..=120_000).contains(&step.timeout_ms)
                    && step.action.is_valid()
                    && !matches!(step.action, Action::Delay { milliseconds } if milliseconds >= step.timeout_ms)
            })
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn valid_bundle_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 255
        && id.contains('.')
        && id.split('.').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// Runtime values are not serializable and Debug is redacted. The owned copies
/// are zeroed on drop; callers remain responsible for their own copies/storage.
#[derive(Default)]
pub struct InputValues(BTreeMap<String, Zeroizing<String>>);
impl InputValues {
    pub fn insert(&mut self, key: impl Into<String>, value: String) {
        self.0.insert(key.into(), Zeroizing::new(value));
    }
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|value| value.as_str())
    }
}
impl fmt::Debug for InputValues {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputValues")
            .field("count", &self.0.len())
            .finish_non_exhaustive()
    }
}
