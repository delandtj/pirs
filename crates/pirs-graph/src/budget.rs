//! Binary-search token-budget packing for tool output, aider-repomap style:
//! instead of a fixed item count (`top(15)`, `truncate(limit)`) or no cap at
//! all, bisect how many of an already-ranked/ordered set of rendered lines
//! to keep so the joined output fits a token budget. A fixed count either
//! wastes budget when items render small, or blows past it when a handful
//! render large (a symbol on a long path, a file with many callers) — and
//! several of pirs-graph's `code_map` actions (`symbol`/`callers`/
//! `file_map`/`blast`) had no cap of any kind before this, so a huge match
//! set could dump an unbounded amount of text into the model's context.

/// No real tokenizer is wired up in pirs-graph, so this is the same chars/4
/// approximation pirs-agent's compaction budget uses (see
/// `pirs_agent::compaction`) — good enough to bound output size, not an
/// exact count, and deliberately not a new cross-crate dependency for one
/// estimate.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count() / 4
}

/// A generous per-tool-result ceiling — most `code_map`/`code_search` calls
/// render far less than this; it exists to bound the pathological cases
/// (thousands of callers, very long paths) rather than to shape the common
/// case.
pub const DEFAULT_TOKEN_BUDGET: usize = 2000;

/// Joins as many of `lines`, in order, as fit within `max_tokens` once
/// newline-joined, via binary search over how many lines to include.
/// `lines` is assumed already sorted best-first — bisection only makes
/// sense when the caller wants a prefix of the most important entries, not
/// an arbitrary subset. Always includes at least the first line when
/// `lines` is non-empty (an oversized-but-present answer beats returning
/// nothing), and appends a note when trailing lines were dropped, so
/// truncation is never silent.
pub fn join_within_budget(lines: &[String], max_tokens: usize) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let render = |k: usize| lines[..k].join("\n");

    let mut best = render(1);
    if estimate_tokens(&best) > max_tokens {
        return best;
    }

    let mut lo = 1usize;
    let mut hi = lines.len();
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let candidate = render(mid);
        if estimate_tokens(&candidate) <= max_tokens {
            lo = mid;
            best = candidate;
        } else {
            hi = mid - 1;
        }
    }

    if lo < lines.len() {
        format!(
            "{best}\n… {} more result(s) omitted to stay within the token budget",
            lines.len() - lo
        )
    } else {
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_empty_output() {
        assert_eq!(join_within_budget(&[], 100), "");
    }

    #[test]
    fn everything_fits_when_well_under_budget() {
        let lines: Vec<String> = (0..5).map(|i| format!("line {i}")).collect();
        let out = join_within_budget(&lines, 1000);
        assert_eq!(out, lines.join("\n"));
    }

    #[test]
    fn bisects_down_to_a_prefix_that_fits_and_notes_the_drop() {
        // Each line is ~12 chars -> ~3 tokens; budget for exactly 3 lines.
        let lines: Vec<String> = (0..20).map(|i| format!("line number {i:02}")).collect();
        let budget = estimate_tokens(&lines[..3].join("\n"));
        let out = join_within_budget(&lines, budget);
        assert!(out.starts_with(&lines[..3].join("\n")), "{out}");
        assert!(
            out.contains("more result(s) omitted"),
            "should note the drop: {out}"
        );
        assert!(!out.contains("line number 19"), "{out}");
    }

    #[test]
    fn a_single_oversized_line_is_still_returned_not_dropped_to_nothing() {
        let huge = "x".repeat(10_000);
        let lines = vec![huge.clone()];
        let out = join_within_budget(&lines, 1);
        assert_eq!(out, huge, "one oversized item beats an empty result");
    }

    #[test]
    fn result_never_exceeds_budget_by_more_than_the_first_line() {
        let lines: Vec<String> = (0..50).map(|i| format!("entry {i:03}")).collect();
        let out = join_within_budget(&lines, 10);
        // Strip the trailing note (if any) before measuring the kept prefix.
        let kept = out.split("\n… ").next().unwrap();
        assert!(
            estimate_tokens(kept) <= 10 || kept == lines[0],
            "kept text should respect the budget once more than one line is included: {kept}"
        );
    }
}
