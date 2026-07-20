#!/usr/bin/env python3
"""monolithic strategy vs qwen3.5-plus (DashScope international, OpenAI-
compatible), across the 8 real/working SWE-bench-lite instances established
in this benchmark (django-11001/sympy-15346 excluded: they need the
agent-discovery fallback and aren't reliable ground-truth signal)."""
import json
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from run_one import run_instance

MODEL = "qwen3.5-plus"
BASE_URL = "https://coding-intl.dashscope.aliyuncs.com/v1"

INSTANCES = [
    "astropy__astropy-6938",
    "scikit-learn__scikit-learn-12471",
    "sphinx-doc__sphinx-7686",
    "astropy__astropy-14182",
    "matplotlib__matplotlib-23562",
    "matplotlib__matplotlib-26011",
    "scikit-learn__scikit-learn-25570",
    "pytest-dev__pytest-5221",
]

if __name__ == "__main__":
    max_turns = int(sys.argv[1]) if len(sys.argv) > 1 else 40
    timeout_s = int(sys.argv[2]) if len(sys.argv) > 2 else 2400
    concurrency = int(sys.argv[3]) if len(sys.argv) > 3 else 2
    out_dir = Path(sys.argv[4]) if len(sys.argv) > 4 else Path(__file__).parent / "results_qwen_monolithic"
    out_dir.mkdir(parents=True, exist_ok=True)

    print(f"monolithic x qwen3.5-plus: {len(INSTANCES)} instances, concurrency={concurrency}", flush=True)

    results = []
    with ThreadPoolExecutor(max_workers=concurrency) as pool:
        futs = {}
        for iid in INSTANCES:
            fut = pool.submit(
                run_instance, iid, MODEL, max_turns, timeout_s, out_dir,
                None, "monolithic-qwen35plus", False, "openai-compat", BASE_URL,
            )
            futs[fut] = iid
        for fut in as_completed(futs):
            iid = futs[fut]
            try:
                r = fut.result()
            except Exception as e:
                r = {"id": iid, "label": "monolithic-qwen35plus", "model": MODEL, "error": str(e), "solved": False}
            print(f"DONE {iid}: solved={r.get('solved')} exit={r.get('exit_code')} elapsed={r.get('elapsed_s')}", flush=True)
            results.append(r)

    summary_path = out_dir / "qwen_monolithic_summary.json"
    summary_path.write_text(json.dumps(results, indent=2))
    print(f"\nwrote summary to {summary_path}")
    solved = sum(1 for r in results if r.get("solved"))
    print(f"\n{solved}/{len(results)} solved")
