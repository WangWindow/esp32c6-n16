//! A tiny test/self-check helper for firmware examples.
//!
//! It is intentionally smaller than a full test harness: the self-test app uses
//! it to run hardware checks at boot, and on-target tests can use the same
//! reporting helpers.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use defmt::{error, info};

/// Collects pass/fail results for a named test group.
pub struct Suite {
    name: &'static str,
    passed: u32,
    failed: u32,
}

impl Suite {
    /// Create a new suite.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            passed: 0,
            failed: 0,
        }
    }

    /// Record a boolean condition.
    pub fn check(&mut self, condition: bool, label: &'static str) -> bool {
        if condition {
            self.passed += 1;
            info!("[{}] PASS {}", self.name, label);
        } else {
            self.failed += 1;
            error!("[{}] FAIL {}", self.name, label);
        }
        condition
    }

    /// Record an equality check.
    pub fn check_eq<T: defmt::Format + PartialEq>(
        &mut self,
        actual: &T,
        expected: &T,
        label: &'static str,
    ) -> bool {
        if actual == expected {
            self.passed += 1;
            info!("[{}] PASS {} = {}", self.name, label, actual);
            true
        } else {
            self.failed += 1;
            error!(
                "[{}] FAIL {}: got {}, expected {}",
                self.name, label, actual, expected
            );
            false
        }
    }

    /// Record a `Result` check, returning the success value if any.
    pub fn check_ok<T, E: defmt::Format>(
        &mut self,
        result: Result<T, E>,
        label: &'static str,
    ) -> Option<T> {
        match result {
            Ok(value) => {
                self.passed += 1;
                info!("[{}] PASS {}", self.name, label);
                Some(value)
            }
            Err(err) => {
                self.failed += 1;
                error!("[{}] FAIL {}: {}", self.name, label, err);
                None
            }
        }
    }

    /// Number of failed checks so far.
    #[must_use]
    pub const fn failed(&self) -> u32 {
        self.failed
    }

    /// Number of passed checks so far.
    #[must_use]
    pub const fn passed(&self) -> u32 {
        self.passed
    }

    /// Print the summary and return whether every check passed.
    pub fn finish(self) -> bool {
        let ok = self.failed == 0;
        if ok {
            info!("[{}] RESULT PASS ({} checks)", self.name, self.passed);
        } else {
            error!(
                "[{}] RESULT FAIL ({} passed, {} failed)",
                self.name, self.passed, self.failed
            );
        }
        ok
    }
}

/// Check a boolean expression in a [`Suite`].
#[macro_export]
macro_rules! check {
    ($suite:expr, $condition:expr, $label:literal) => {
        $suite.check($condition, $label)
    };
}

/// Check equality in a [`Suite`].
#[macro_export]
macro_rules! check_eq {
    ($suite:expr, $actual:expr, $expected:expr, $label:literal) => {
        $suite.check_eq(&$actual, &$expected, $label)
    };
}

/// Check a `Result` in a [`Suite`].
#[macro_export]
macro_rules! check_ok {
    ($suite:expr, $result:expr, $label:literal) => {
        $suite.check_ok($result, $label)
    };
}
