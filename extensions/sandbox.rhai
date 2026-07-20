// sandbox.rhai — wraps `bash` in an OS-level sandbox: read-only filesystem
// outside the working directory, no network access.
//
// This is the single most repeated finding from a comparative architecture
// study of Grok Build, pi (the project pirs itself is a Rust port of),
// qwen-code, mistral-vibe, cline, and aider: pirs runs shell commands fully
// unconfined, while several of those projects sandbox by construction
// (Grok Build: Landlock+bwrap+Seatbelt; qwen-code: Docker/Podman+Seatbelt)
// and — notably — pi implements it exactly this way: as an extension that
// overrides its own bash tool, not a core change
// (packages/coding-agent/examples/extensions/sandbox/index.ts). This pack
// is pirs's rhai-native equivalent of that.
//
// It works by re-registering a tool named "bash" — a later-registered tool
// sharing a native tool's name wins on both dispatch and in the schema sent
// to the model (see pirs-agent's tool_defs/prepare_call), so this pack
// transparently replaces the built-in, unsandboxed bash whenever it's
// loaded from .pirs/extensions/ or ~/.pirs/extensions/.
//
// Policy (fixed for this first cut, not yet configurable):
//   - Linux, primary: bubblewrap (bwrap) — the whole filesystem is
//     read-only except the working directory (read-write) and a fresh,
//     empty /tmp; network is fully disabled (--unshare-net).
//   - Linux, fallback: if bwrap is missing, OR bwrap itself fails to start
//     the sandbox (see the AppArmor note below — a real, common case, not
//     hypothetical), and docker or podman is on PATH, this pack
//     transparently falls back to a container-based sandbox instead of
//     failing loud: bind-mount the host root read-only plus the working
//     directory read-write, `chroot` into it, `--network none`, then
//     `chown` any files the command created back to the calling user (the
//     container runs as root — required for chroot's CAP_SYS_CHROOT — so
//     without this step created files would be owned by root on the host).
//     This sidesteps bwrap's unprivileged-userns restriction entirely
//     because the container engine's already-privileged daemon sets up the
//     namespaces on the caller's behalf, rather than the caller creating an
//     unprivileged one itself. The output is annotated so it's never
//     silently a different sandbox than the model/user might expect.
//   - macOS (sandbox-exec): same intent via a generated Seatbelt profile.
//     NOTE: this path is written from documented Seatbelt syntax but has
//     not been exercised live (this pack was authored on Linux) — treat it
//     as a starting point to verify on a real Mac, not a proven quantity.
//   - Anything else, or no sandboxing mechanism available at all: FAILS
//     LOUD with an actionable message. It never silently falls through to
//     running the command unsandboxed — that would be exactly the kind of
//     confident-wrong behavior this whole comparative study flagged as the
//     failure mode to design against.
//
// `background`/`auto_restart` (the native bash tool's job-management modes)
// are not supported here — sandboxing a long-running background job needs
// the native job registry this pack doesn't reimplement. Remove this pack
// (or ask the user to) for tasks that need those.
//
// Known environment constraint, confirmed while building this: on Ubuntu
// 23.10+/24.04+, `kernel.apparmor_restrict_unprivileged_userns=1` (the
// default) blocks bwrap's unprivileged user-namespace creation entirely,
// even with `--unshare-user-try` — every bash call fails with something
// like "setting up uid map: Permission denied" or "loopback: Failed
// RTM_NEWADDR: Operation not permitted" before the wrapped command ever
// runs. The docker/podman fallback above exists specifically because of
// this: it's confirmed to work in exactly this environment. Without
// docker/podman on PATH, the remaining options are granting bwrap an
// AppArmor exception, running `sudo sysctl -w
// kernel.apparmor_restrict_unprivileged_userns=0` (a real host security
// posture change — decide deliberately, don't cargo-cult it), or removing
// this pack to go back to unsandboxed bash.

register_tool("bash", "Run a bash command, sandboxed: the filesystem is read-only outside the working directory, and there is no network access. stdout and stderr are combined. background/auto_restart are not supported in sandboxed mode.", #{
    type: "object",
    properties: #{
        command: #{ type: "string", description: "Command to run in bash" },
        timeout: #{ type: "integer", description: "Timeout in seconds; the process tree is killed on expiry" },
        background: #{ type: "boolean", description: "Not supported in sandboxed mode — remove sandbox.rhai for background jobs" },
        auto_restart: #{ type: "boolean", description: "Not supported in sandboxed mode" }
    },
    required: ["command"]
});

// Which container engine (if any) is available for the Linux fallback path.
// Docker is checked first only because it's the more common install; either
// is equally suitable here.
fn container_engine() {
    if exec("command -v docker", 5).code == 0 {
        return "docker";
    }
    if exec("command -v podman", 5).code == 0 {
        return "podman";
    }
    ""
}

// Bind-mount the host root read-only, the working directory read-write over
// it, chroot in, and disable networking. Runs as root inside the container
// (chroot needs CAP_SYS_CHROOT, which a non-root `--user` loses), so any
// files the command creates come out root-owned on the host unless fixed up
// — the trailing `chown` does that, and `ec=$?`/`exit $ec` around it
// preserve the wrapped command's own exit code rather than the chown's.
fn run_container_sandbox(engine, cwd, script_path, timeout) {
    let uid = exec("id -u", 5).output;
    uid.trim();
    let gid = exec("id -g", 5).output;
    gid.trim();
    let cmd = `${engine} run --rm --network none -v /:/hostfs:ro -v "${cwd}":"/hostfs${cwd}":rw alpine:3 chroot /hostfs sh -c "cd ${cwd} && sh ${script_path}; ec=\$?; chown -R ${uid}:${gid} ${cwd} 2>/dev/null; exit \$ec"`;
    exec(cmd, timeout)
}

// A `bwrap:`/`sandbox-exec:`-prefixed line means the sandboxing tool itself
// failed to set up (namespaces/binds/profile) before the wrapped command
// ever ran — as opposed to the wrapped command failing, whose output has no
// such prefix. The exact message varies by flag combination and
// kernel/AppArmor version, so this checks the prefix rather than one
// hardcoded string.
fn is_sandbox_setup_failure(out) {
    out.contains("bwrap:") || out.contains("sandbox-exec:")
}

fn format_sandbox_result(result, timeout, note) {
    let out = result.output;
    if note != "" {
        out = `${out}\n[${note}]`;
    }
    if result.timedOut {
        return `${out}\n[sandboxed command timed out after ${timeout}s]`;
    }
    if result.code != 0 {
        return `${out}\n[exit code ${result.code}]`;
    }
    out
}

fn tool_bash(args) {
    if (args.background ?? false) || (args.auto_restart ?? false) {
        return "sandboxed bash does not support background/auto_restart. Remove sandbox.rhai from .pirs/extensions/ (or ~/.pirs/extensions/) to run this command unsandboxed with background-job support.";
    }
    let timeout = args.timeout ?? 120;

    let cwd = exec("pwd", 5).output;
    cwd.trim();
    let uname = exec("uname", 5).output;
    uname.trim();

    // The command is written to a script file and executed by path rather
    // than embedded as a nested quoted string in the wrapper command — that
    // sidesteps an entire class of quote-escaping bugs a naive
    // `bwrap ... -- sh -c "${command}"` string-build would risk once the
    // command itself contains quotes.
    let script_path = `${cwd}/.pirs/sandbox-tmp/cmd-${now_millis()}.sh`;
    fs_write(script_path, args.command);

    if uname == "Linux" {
        if exec("command -v bwrap", 5).code == 0 {
            let wrapped = `bwrap --die-with-parent --unshare-net --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp --bind "${cwd}" "${cwd}" -- sh "${script_path}"`;
            let result = exec(wrapped, timeout);
            if !(result.code != 0 && is_sandbox_setup_failure(result.output)) {
                exec(`rm -rf "${cwd}/.pirs/sandbox-tmp"`, 5);
                return format_sandbox_result(result, timeout, "");
            }
            // bwrap itself failed to start (commonly
            // kernel.apparmor_restrict_unprivileged_userns=1) — fall back
            // to a container engine, which isn't subject to that
            // restriction.
            let engine = container_engine();
            if engine == "" {
                exec(`rm -rf "${cwd}/.pirs/sandbox-tmp"`, 5);
                return `${result.output}\n[bwrap sandbox setup failed before the command ran — commonly kernel.apparmor_restrict_unprivileged_userns=1 (the Ubuntu 23.10+/24.04+ default) blocking bwrap's unprivileged user namespace; no docker/podman fallback available either (neither found on PATH) — see the note at the top of sandbox.rhai]`;
            }
            let cresult = run_container_sandbox(engine, cwd, script_path, timeout);
            exec(`rm -rf "${cwd}/.pirs/sandbox-tmp"`, 5);
            return format_sandbox_result(cresult, timeout, `sandboxed via ${engine} fallback — bwrap failed to start; see sandbox.rhai`);
        }
        let engine = container_engine();
        if engine == "" {
            exec(`rm -f "${script_path}"`, 5);
            return "sandbox.rhai requires bubblewrap, docker, or podman on Linux. Install one (e.g. `apt install bubblewrap`) or remove sandbox.rhai from .pirs/extensions/ to run unsandboxed.";
        }
        let cresult = run_container_sandbox(engine, cwd, script_path, timeout);
        exec(`rm -rf "${cwd}/.pirs/sandbox-tmp"`, 5);
        return format_sandbox_result(cresult, timeout, `sandboxed via ${engine} (bwrap not installed)`);
    } else if uname == "Darwin" {
        if exec("command -v sandbox-exec", 5).code != 0 {
            exec(`rm -f "${script_path}"`, 5);
            return "sandbox.rhai requires sandbox-exec on macOS (normally preinstalled). Remove sandbox.rhai from .pirs/extensions/ to run unsandboxed if it's unavailable.";
        }
        let profile_path = `${cwd}/.pirs/sandbox-tmp/profile-${now_millis()}.sb`;
        let profile = `(version 1)
(deny default)
(allow process-fork)
(allow process-exec)
(allow signal (target self))
(allow sysctl-read)
(allow file-read*)
(allow file-write* (subpath "${cwd}"))
(allow file-write* (subpath "/tmp"))
(allow file-write* (subpath "/private/tmp"))
`;
        fs_write(profile_path, profile);
        let wrapped = `sandbox-exec -f "${profile_path}" sh "${script_path}"`;
        let result = exec(wrapped, timeout);
        exec(`rm -rf "${cwd}/.pirs/sandbox-tmp"`, 5);
        format_sandbox_result(result, timeout, "")
    } else {
        exec(`rm -f "${script_path}"`, 5);
        `sandbox.rhai does not support platform "${uname}". Remove sandbox.rhai from .pirs/extensions/ to run unsandboxed.`
    }
}
