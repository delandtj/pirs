# pirs-work

Terminal **coding work agent** in the class of Claude Code, Codex, Qoder, and Kimi Code.

## What it is

Thin defaults over pirs:

- workspace cwd + coding tools (`read` / `edit` / `bash` / `grep` / …)
- default strategy **`plan-exec`**
- default **`--model qwen3.5-plus`** (exec) + **`--plan-model deepseek-v4-pro`** (plan)
- prefers the `pirs` binary for true multi-phase strategy + registry routing

## Usage

```bash
cargo build -p pirs-work -p pirs
export PATH="$(dirname $(which pirs 2>/dev/null || echo /home/driver/hero/build/target/debug/pirs)):$PATH"

pirs-work -C /path/to/repo "add a --json flag to the CLI"
pirs-work --model qwen3.5-plus --plan-model deepseek-v4-flash --strategy plan-exec "fix …"
```

Interactive TUI (full product):

```bash
pirs --mode tui --strategy plan-exec --model qwen3.5-plus --plan-model deepseek-v4-pro
```

Keys: `~/.pirs/secrets.env` + `~/.pirs/config.toml` backends (see main pirs README).
