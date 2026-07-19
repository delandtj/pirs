//! Verify-and-retry: run an attempt, check a gate, and re-run with the failure
//! fed back until it passes or attempts run out.
//!
//! This is the loop that turns a one-shot strategy into a self-correcting one:
//! after each attempt a verification (build/tests) runs, and if it fails its
//! output becomes the next attempt's `{verdict}` so the strategy can re-plan
//! against the real error instead of guessing. The orchestration is a pure
//! function of two async closures — the attempt and the gate — so it is testable
//! without an agent or a real command.

use std::future::Future;

/// The outcome of a gated run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// The gate passed on the attempt numbered `on_attempt` (1-based).
    Passed { on_attempt: u32 },
    /// Every attempt was used and the gate still failed; carries the last
    /// failure text.
    Exhausted { last_failure: String },
}

impl GateOutcome {
    pub fn passed(&self) -> bool {
        matches!(self, GateOutcome::Passed { .. })
    }
}

/// Run `attempt` up to `max_attempts` times, calling `verify` after each.
///
/// `attempt` receives the previous failure verdict (`None` on the first try) so
/// it can render `{verdict}`. `verify` returns `None` when the gate passes, or
/// `Some(failure_text)` when it fails. Stops at the first pass; otherwise runs
/// out of attempts and returns the last failure. An `attempt` error aborts
/// immediately (propagated) — that's an agent failure, not a gate failure.
///
/// `max_attempts` is clamped to at least 1.
pub async fn run_gated<A, AF, V, VF>(
    max_attempts: u32,
    mut attempt: A,
    mut verify: V,
) -> anyhow::Result<GateOutcome>
where
    A: FnMut(Option<String>) -> AF,
    AF: Future<Output = anyhow::Result<()>>,
    V: FnMut() -> VF,
    VF: Future<Output = Option<String>>,
{
    let max = max_attempts.max(1);
    let mut verdict: Option<String> = None;
    for i in 1..=max {
        attempt(verdict.clone()).await?;
        match verify().await {
            None => return Ok(GateOutcome::Passed { on_attempt: i }),
            Some(failure) => verdict = Some(failure),
        }
    }
    Ok(GateOutcome::Exhausted {
        last_failure: verdict.unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[tokio::test]
    async fn passes_on_first_attempt_runs_once() {
        let attempts = RefCell::new(0);
        let out = run_gated(
            3,
            |_verdict| async {
                *attempts.borrow_mut() += 1;
                Ok(())
            },
            || async { None }, // gate always passes
        )
        .await
        .unwrap();
        assert_eq!(out, GateOutcome::Passed { on_attempt: 1 });
        assert_eq!(*attempts.borrow(), 1, "must not retry after a pass");
    }

    #[tokio::test]
    async fn retries_with_verdict_then_passes() {
        // Fail the first two gates, pass the third; assert the prior failure is
        // threaded into the next attempt as the verdict.
        let seen_verdicts: RefCell<Vec<Option<String>>> = RefCell::new(Vec::new());
        let gate_calls = RefCell::new(0);
        let out = run_gated(
            5,
            |verdict| {
                seen_verdicts.borrow_mut().push(verdict);
                async { Ok(()) }
            },
            || {
                *gate_calls.borrow_mut() += 1;
                let n = *gate_calls.borrow();
                async move {
                    if n < 3 {
                        Some(format!("failure #{n}"))
                    } else {
                        None
                    }
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(out, GateOutcome::Passed { on_attempt: 3 });
        // First attempt has no verdict; the next two carry the prior failures.
        assert_eq!(
            *seen_verdicts.borrow(),
            vec![
                None,
                Some("failure #1".to_string()),
                Some("failure #2".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn exhausts_attempts_and_returns_last_failure() {
        let attempts = RefCell::new(0);
        let out = run_gated(
            2,
            |_v| {
                *attempts.borrow_mut() += 1;
                async { Ok(()) }
            },
            || async { Some("still red".to_string()) }, // never passes
        )
        .await
        .unwrap();
        assert_eq!(
            out,
            GateOutcome::Exhausted {
                last_failure: "still red".to_string()
            }
        );
        assert_eq!(*attempts.borrow(), 2, "used exactly max_attempts");
        assert!(!out.passed());
    }

    #[tokio::test]
    async fn zero_max_attempts_is_clamped_to_one() {
        let attempts = RefCell::new(0);
        let out = run_gated(
            0,
            |_v| {
                *attempts.borrow_mut() += 1;
                async { Ok(()) }
            },
            || async { None },
        )
        .await
        .unwrap();
        assert_eq!(*attempts.borrow(), 1);
        assert!(out.passed());
    }

    #[tokio::test]
    async fn attempt_error_aborts_immediately() {
        let out = run_gated(
            3,
            |_v| async { anyhow::bail!("agent exploded") },
            || async { None },
        )
        .await;
        assert!(out.is_err());
        assert!(out.unwrap_err().to_string().contains("agent exploded"));
    }
}
