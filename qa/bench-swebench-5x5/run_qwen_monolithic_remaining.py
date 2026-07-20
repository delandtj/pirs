#!/usr/bin/env python3
"""monolithic strategy vs qwen3.5-plus across the remaining 42 of the 50
pre-pulled SWE-bench-lite docker images (8 already run in the first batch)."""
import json
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from run_one import run_instance

MODEL = "qwen3.5-plus"
BASE_URL = "https://coding-intl.dashscope.aliyuncs.com/v1"

INSTANCES = [
    "astropy__astropy-12907",
    "django__django-11583",
    "django__django-11742",
    "django__django-11905",
    "django__django-14608",
    "django__django-15213",
    "django__django-15695",
    "django__django-15781",
    "django__django-16046",
    "sympy__sympy-13177",
    "sympy__sympy-13647",
    "sympy__sympy-15346",
    "sympy__sympy-15678",
    "sympy__sympy-16106",
    "astropy__astropy-14365",
    "astropy__astropy-14995",
    "astropy__astropy-7746",
    "django__django-10914",
    "django__django-10924",
    "django__django-11001",
    "django__django-11019",
    "django__django-11039",
    "django__django-11049",
    "django__django-11099",
    "django__django-11133",
    "django__django-11179",
    "django__django-11283",
    "django__django-11422",
    "django__django-11564",
    "django__django-11620",
    "django__django-11630",
    "django__django-11797",
    "django__django-11815",
    "django__django-11848",
    "django__django-11910",
    "django__django-11964",
    "django__django-11999",
    "django__django-12113",
    "django__django-12125",
    "django__django-12184",
    "django__django-12284",
    "django__django-12286",
]

if __name__ == "__main__":
    max_turns = int(sys.argv[1]) if len(sys.argv) > 1 else 40
    timeout_s = int(sys.argv[2]) if len(sys.argv) > 2 else 2400
    concurrency = int(sys.argv[3]) if len(sys.argv) > 3 else 2
    out_dir = Path(sys.argv[4]) if len(sys.argv) > 4 else Path(__file__).parent / "results_qwen_monolithic_remaining"
    out_dir.mkdir(parents=True, exist_ok=True)

    print(f"monolithic x qwen3.5-plus (remaining): {len(INSTANCES)} instances, concurrency={concurrency}", flush=True)

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

    summary_path = out_dir / "qwen_monolithic_remaining_summary.json"
    summary_path.write_text(json.dumps(results, indent=2))
    print(f"\nwrote summary to {summary_path}")
    solved = sum(1 for r in results if r.get("solved"))
    print(f"\n{solved}/{len(results)} solved")
