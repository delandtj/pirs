#!/usr/bin/env python3
"""Re-run monolithic (post system-prompt fix) vs no-strategy on the 3 real
instances (matplotlib-23562 and pytest-5221 are excluded: pre-existing
ReproFailed harness issue, unrelated to strategy, confirmed identical across
all 5 strategies in the original 5x5 run)."""
import json
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from run_one import run_instance

BASE_MODEL = "deepseek-v4-flash"
STRATEGIES = [
    {"label": "monolithic-v2", "strategy_script": None, "no_strategy": False},
    {"label": "no-strategy", "strategy_script": None, "no_strategy": True},
]
INSTANCES = [
    "astropy__astropy-6938",
    "scikit-learn__scikit-learn-12471",
    "sphinx-doc__sphinx-7686",
]

if __name__ == "__main__":
    max_turns = int(sys.argv[1]) if len(sys.argv) > 1 else 40
    timeout_s = int(sys.argv[2]) if len(sys.argv) > 2 else 2400
    concurrency = int(sys.argv[3]) if len(sys.argv) > 3 else 2
    out_dir = Path(sys.argv[4]) if len(sys.argv) > 4 else Path(__file__).parent / "results_rerun"
    out_dir.mkdir(parents=True, exist_ok=True)

    jobs = [(iid, strat) for iid in INSTANCES for strat in STRATEGIES]
    print(f"rerun: {len(INSTANCES)} instances x {len(STRATEGIES)} strategies = {len(jobs)} runs, concurrency={concurrency}", flush=True)

    results = []
    with ThreadPoolExecutor(max_workers=concurrency) as pool:
        futs = {}
        for iid, strat in jobs:
            fut = pool.submit(
                run_instance, iid, BASE_MODEL, max_turns, timeout_s, out_dir,
                strat["strategy_script"], strat["label"], strat["no_strategy"],
            )
            futs[fut] = (iid, strat["label"])
        for fut in as_completed(futs):
            iid, label = futs[fut]
            try:
                r = fut.result()
            except Exception as e:
                r = {"id": iid, "label": label, "model": BASE_MODEL, "error": str(e), "solved": False}
            print(f"DONE {iid} [{label}]: solved={r.get('solved')} exit={r.get('exit_code')} elapsed={r.get('elapsed_s')}", flush=True)
            results.append(r)

    summary_path = out_dir / "rerun_summary.json"
    summary_path.write_text(json.dumps(results, indent=2))
    print(f"\nwrote summary to {summary_path}")
