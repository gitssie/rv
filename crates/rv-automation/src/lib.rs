//! Host-side workflow primitives. No application-specific registration rules,
//! credentials, transport, GUI, or persistence live here.
//!
//! The caller owns the device's exclusive input lease for the entire `run`
//! future. A runner serializes its own runs, but does not arbitrate other RV
//! controllers. Do not abort a run to cancel it: use [`RunControl::cancel`] and
//! await its terminal progress. A cancelled/timed-out mutation may have executed.

mod device;
mod model;
mod runner;

pub use device::*;
pub use model::*;
pub use runner::*;

#[cfg(test)]
mod tests;
