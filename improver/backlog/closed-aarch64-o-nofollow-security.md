# aarch64-o-nofollow-security

- **Stage:** verify (security, bypasses scoring per continuous-improvement.md)
- **Evidence:** investigator (kitten) 2026-10-17, reproduced in the
  hwatu-builder aarch64 container: hardcoded `O_NOFOLLOW = 0o400000` is the
  x86 Linux value; aarch64 Linux uses `0o100000` (0x8000, per
  bits/fcntl.h `__O_NOFOLLOW 0100000`). Functional probe: open with
  0o400000 FOLLOWED a symlink and overwrote the target; os.O_NOFOLLOW
  refused with ELOOP. Symlink-overwrite protection is therefore silently
  absent in every aarch64 Linux build. Test
  `remote::tests::materialization_refuses_symlink_destinations` fails
  there (assert at crates/hwatu/src/remote.rs:629).
- **Affected sites:**
  - crates/hwatu/src/remote.rs:22-24 (per-OS consts, x86-only comment)
  - crates/hwatud/src/private_files.rs:21 (ungated 0o400000, wrong on
    macOS AND aarch64 Linux)
  - crates/hwatu/src/verify_job.rs:1069-1070 (same ungated pattern;
    d33fd05 fixed only remote.rs)
- **Fix direction:** use `libc::O_NOFOLLOW`/`libc::O_NONBLOCK` (arch- and
  OS-correct) at all three sites, or exhaustive per-arch cfg constants.
  Prefer libc: hwatud already links platform libs; check hwatu's
  dependency policy (deny.toml) before adding libc to the thin client.
- **Acceptance check:** `scripts/improver/accept/nofollow-arch.sh` —
  cargo test passes in the aarch64 Linux container AND on macOS; a repo
  grep shows no hardcoded 0o400000/0o4000 open-flag literals remain.
- **Severity:** security (symlink redirect to arbitrary overwrite);
  bypasses rubric scoring.
