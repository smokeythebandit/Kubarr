# Test ledger

Run from the repository root. The generator is Python standard-library only and
never runs tests, accesses the network, or reads application credentials.

For one command covering all safe local lanes, including every Rust library
test (not merely the Settings integration target), run:

```sh
bash tests/report/run-local.sh
```

It runs isolated SQLite library/Settings/registration targets, Vitest and the
strict mocked browser suite; it keeps the run's raw logs alongside the ignored
ledger. Node 26 is the supported frontend runtime. This runner invokes the
already installed test executables directly where a local older Node prevents
pnpm's engine gate; it does not alter package requirements.

```sh
python3 -B -m unittest discover -s tests/report -p 'test_*.py'
python3 tests/report/generate.py \
  --vitest code/frontend/test-results/unit.json \
  --playwright browser=code/frontend/test-results/browser-results.json \
  --output code/frontend/test-results/ledger
```

Open `code/frontend/test-results/ledger/index.html` locally. The adjacent
`results.json` contains the exact observed scenario titles, sources, lane,
duration/start time (when the runner supplies them), generator timestamp, git
commit and CI run ID. Use `--rust /path/to/cargo-test.log` for each targeted
Rust test run; use `bash tests/report/run-rust.sh /tmp/kubarr-rust-lib.log
cargo test --locked --lib` to record both output and the actual command exit
status in `/tmp/kubarr-rust-lib.log.exit`. Missing markers show `not-run` command
outcomes even if a log contains passing test lines; nonzero exits show `failed`,
including compilation failures with no test rows and failures after all test
rows were printed. CI captures a separate outcome marker for each Rust target.
Use `--playwright live=code/frontend/test-results/real-PHASE.json`
once per phase after the guarded disposable-cluster harness has run. The
`--lane-result api-live=passed` or `--lane-result api-live=failed` option
records only the outcome of the shell API acceptance harness, without
inventing per-scenario pass counts. It must reflect an actual harness run.
The `playwright-report/browser/` and `playwright-report/real/PHASE/` HTML reports
remain the native attachment/screenshot entrypoints; ledger links work when
the output lives at `code/frontend/test-results/ledger/`. Retain both directories
when moving artifacts.
Run discovery (`playwright test --list`) before the actual test run: the JSON
reporter also writes a discovery-only file, which the ledger marks `not-run`.

`scenarios.json` is the explicit Settings-tab and nonsettings requirements
inventory. A requirement is passed only if its explicitly mapped **exact test
title** in that lane actually ran and passed; object entries allow one real
journey to substantiate multiple precise tab-level requirements. If the lane
ran but the requirement lacks a matching test, its row is `not-covered`; if
the lane was not supplied, its row
is `not-run`. Runner-reported skipped/failed tests stay skipped/failed.
Observed test rows are always included even when they are not in the
requirements inventory. The inventory intentionally lists gaps; neither a
mocked browser pass nor a profile write proves real DNS, TLS issuance, OAuth,
external notification delivery or VPN transport.

CI uploads independent frontend, backend, CLI and PostgreSQL audit ledgers from
`fast-test.yml`, API-harness and real browser ledgers from `acceptance.yml`,
and a VPN lab outcome from `vpn-acceptance.yml`. Each job shows only the inputs
actually run there; merge lanes locally by passing all JSON/log inputs to one
generator invocation. Do not infer successful tests from a missing artifact.
