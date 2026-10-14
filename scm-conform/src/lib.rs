//! The conformance crate's library half: the differential fuzzer.
//!
//! Two implementations of one language is the differential method (plan
//! §5), and the suites exercise it on programs people wrote. This half
//! exercises it on programs nobody wrote: [`fuzz::Gen`] builds random,
//! mostly well-typed programs from a grammar that reaches every form the
//! tiers handle specially, and [`fuzz::compare`] runs each under T0 and T1
//! -- and T2 when it is installed -- and reports any difference in value,
//! output or error message.

pub mod fuzz;
