#!/usr/bin/env python3
"""Generate a static, local-only test ledger. Missing inputs never imply success."""
import argparse
import datetime as dt
import html
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
FRONTEND = ROOT / 'code/frontend'
INVENTORY = Path(__file__).with_name('scenarios.json')
GROUPS = {
    '03-settings.spec.ts': 'general', '07-storage.spec.ts': 'storage',
    '14-vpn.spec.ts': 'vpn', '15-notifications.spec.ts': 'notifications',
    '17-domains.spec.ts': 'domains', 'settings-vpn.spec.ts': 'vpn',
    'settings-accounts.spec.ts': 'users', '18-settings-access.spec.ts': 'general',
}


def group(file, title):
    name = Path(file).name
    if name == 'settings-vpn.spec.ts':
        if 'registration' in title: return 'general'
        if 'notification' in title: return 'notifications'
        if 'theme' in title: return 'account'
    if name == 'settings-accounts.spec.ts':
        for word, section in [('registration', 'general'), ('approval', 'pending'),
                              ('invite', 'invites'), ('role', 'permissions'), ('audit', 'audit')]:
            if word in title.lower(): return section
    if name == 'settings-profiles.spec.ts':
        return 'domains' if 'domain inventory' in title else 'ddns/letsencrypt'
    if name == '19-settings-profiles.spec.ts':
        return 'ddns' if 'DNS profile' in title else 'letsencrypt'
    if name.startswith('settings_'): return 'general'
    if name.startswith('ddns_'): return 'ddns'
    if name.startswith('domain_'): return 'domains'
    if name.startswith('letsencrypt_'): return 'letsencrypt'
    return GROUPS.get(name, re.sub(r'^\d+-', '', name).split('.')[0].split('_')[0])


def playwright(path, lane):
    data = json.loads(path.read_text())
    records = []

    def walk(suites, source=''):
        for suite in suites:
            file = suite.get('file') or source
            for spec in suite.get('specs', []):
                for test in spec.get('tests', []):
                    results = test.get('results', [])
                    status = test.get('status', 'skipped')
                    if status in ('expected', 'flaky'):
                        # "expected" also describes an expected failure (test.fail).
                        # Only an actual pass with a passing expectation is a pass.
                        actual = results[-1].get('status') if results else None
                        expected = test.get('expectedStatus', 'passed')
                        if expected == 'failed' or actual in ('failed', 'timedOut', 'interrupted'):
                            status = 'failed'
                        elif expected == 'skipped' or actual == 'skipped':
                            status = 'skipped'
                        elif actual == 'passed' or (results and actual is None and expected == 'passed'):
                            status = 'passed'
                        else:
                            status = 'not-run'
                    elif status == 'unexpected':
                        status = 'failed'
                    if status == 'skipped' and not results and not any(
                            a.get('type') in ('skip', 'fixme') for a in test.get('annotations', [])):
                        status = 'not-run'  # Playwright --list includes discovered tests without results.
                    if status not in ('passed', 'failed', 'skipped'): status = 'not-run'
                    records.append(dict(lane=lane, section=group(file, spec['title']),
                                        scenario=spec['title'], status=status, source=file,
                                        run_at=results[0].get('startTime') if results else None,
                                        duration_ms=sum(r.get('duration', 0) for r in results),
                                        evidence=[a.get('name') for r in results for a in r.get('attachments', [])
                                                  if a.get('name') in ('settings-inventory', 'domain-inventory', 'sanitized-failure')]))
            walk(suite.get('suites', []), file)
    walk(data.get('suites', []))
    if data.get('errors'):
        records.append(dict(lane=lane, section='runner', scenario='Playwright runner error (see native report)',
                            status='failed', source=path.name))
    return records


def vitest(path):
    records = []
    data = json.loads(path.read_text())
    for suite in data.get('testResults', []):
        source = suite.get('name', '')
        for result in suite.get('assertionResults', []):
            status = result.get('status', 'pending')
            records.append(dict(lane='unit', section=group(source, result['title']),
                                scenario=result['title'], status=status if status in ('passed', 'failed') else 'skipped',
                                source=source, run_at=dt.datetime.fromtimestamp(
                                    suite['startTime'] / 1000, dt.timezone.utc).isoformat() if suite.get('startTime') else None))
    if data.get('success') is False and not any(r['status'] == 'failed' for r in records):
        records.append(dict(lane='unit', section='runner', scenario='Vitest runner failed before test result',
                            status='failed', source=path.name))
    return records


def rust(path):
    records = []
    target = path.name
    captured_at = dt.datetime.fromtimestamp(path.stat().st_mtime, dt.timezone.utc).isoformat()
    pending = False
    for line in path.read_text().splitlines():
        binary = re.search(r'Running (?:unittests |tests/)([^ ()]+)', line)
        if binary: target = binary.group(1)
        planned = re.match(r'^running (\d+) tests?$', line)
        if planned:
            if pending:
                records.append(dict(lane='api', section='runner', scenario='Rust target interrupted before summary',
                                    status='failed', source=target, run_at=captured_at))
            pending = True
        match = re.match(r'^test (.+) \.\.\. (ok|FAILED|ignored)$', line)
        if match:
            name, result = match.groups()
            section = 'general' if 'settings' in path.stem else group(path.stem, name)
            records.append(dict(lane='api', section=section, scenario=name,
                                status={'ok': 'passed', 'FAILED': 'failed', 'ignored': 'skipped'}[result],
                                source=target, run_at=captured_at))
        if re.match(r'^test result: (ok|FAILED)\.', line): pending = False
    if pending:
        records.append(dict(lane='api', section='runner', scenario='Rust target interrupted before summary',
                            status='failed', source=path.name, run_at=captured_at))
    return records


def rust_outcome(path):
    """An exit marker is required: cargo can fail before printing any test rows."""
    marker = Path(f'{path}.exit')
    if not marker.is_file(): return None
    value = marker.read_text().strip()
    if not re.fullmatch(r'\d+', value): raise ValueError(f'Invalid Rust exit marker: {marker}')
    return int(value)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--playwright', action='append', default=[], metavar='LANE=JSON')
    parser.add_argument('--vitest', action='append', default=[], type=Path)
    parser.add_argument('--rust', action='append', default=[], type=Path)
    parser.add_argument('--lane-result', action='append', default=[], metavar='LANE=STATUS',
                        help='Outcome of a harness without per-scenario JSON (passed/failed only)')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    records, inputs, links, commands = [], [], {}, []
    seen_inputs = set()
    for value in args.playwright:
        lane, sep, filename = value.partition('=')
        if not sep or lane not in ('browser', 'live'): parser.error('--playwright requires browser=JSON or live=JSON')
        path = Path(filename)
        if path.is_file():
            if path.resolve() in seen_inputs: parser.error(f'Duplicate Playwright input: {path}')
            seen_inputs.add(path.resolve())
            lane_records = playwright(path, lane)
            for record in lane_records: record['input'] = str(path)
            records.extend(lane_records)
            inputs.append(str(path))
            phase = path.stem.removeprefix('real-')
            native = FRONTEND / 'playwright-report' / ('browser' if lane == 'browser' else f'real/{phase}') / 'index.html'
            if native.is_file(): links[str(path)] = os.path.relpath(native, args.output)
    for path in args.vitest:
        if path.is_file():
            if path.resolve() in seen_inputs: parser.error(f'Duplicate input: {path}')
            seen_inputs.add(path.resolve())
            records.extend(vitest(path)); inputs.append(str(path))
    for path in args.rust:
        if path.resolve() in seen_inputs: parser.error(f'Duplicate input: {path}')
        seen_inputs.add(path.resolve())
        try: exit_code = rust_outcome(path)
        except ValueError as error: parser.error(str(error))
        command_status = 'failed' if exit_code is not None and exit_code != 0 else 'passed' if exit_code == 0 and path.is_file() else 'not-run'
        commands.append(dict(lane='api', log=str(path), exit_code=exit_code, status=command_status))
        rust_records = rust(path) if path.is_file() else []
        if path.is_file(): inputs.append(str(path))
        records.extend(rust_records)
        if command_status != 'passed' and not any(r['status'] == 'failed' for r in rust_records):
            records.append(dict(lane='api', section='runner', scenario='Rust command failed before test result' if exit_code is not None else 'Rust command outcome unavailable',
                                status=command_status, source=path.name))
    for item in args.lane_result:
        lane, sep, status = item.partition('=')
        if not sep or not re.fullmatch(r'[a-z][a-z-]*', lane) or status not in ('passed', 'failed'):
            parser.error('--lane-result requires LANE=passed or LANE=failed')
        records.append(dict(lane=lane, section='acceptance', scenario='harness outcome (no per-scenario results)',
                            status=status, source='workflow step outcome'))
    inventory = json.loads(INVENTORY.read_text())
    observed = list(records)
    present_lanes = {r['lane'] for r in observed}
    for section_type, sections in inventory.items():
        for section, scenarios in sections.items():
            for entry in scenarios:
                scenario = entry if isinstance(entry, str) else entry['name']
                # Requirements only count as covered by the exact named test, never a
                # similar title, a page render, or a result from another lane.
                for lane in ('unit', 'api', 'browser', 'live'):
                    title = entry.get(lane) if isinstance(entry, dict) else scenario
                    exact = next((r for r in observed if r['lane'] == lane and r['scenario'] == title), None) if title else None
                    records.append(dict(lane=lane, section=section, scenario=scenario, kind='requirement',
                                        test=title if exact else None,
                                        status=exact['status'] if exact else 'not-covered' if lane in present_lanes else 'not-run',
                                        source=exact['source'] if exact else f'{section_type}/{section} inventory'))
    commit = subprocess.run(['git', 'rev-parse', '--short', 'HEAD'], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    now = dt.datetime.now(dt.timezone.utc).isoformat()
    output = dict(schema=1, generated_at=now, commit=commit, environment={
        'ci': bool(os.getenv('CI')), 'run_id': os.getenv('GITHUB_RUN_ID') or os.getenv('ACCEPTANCE_RUN_ID'),
        'node': os.getenv('NODE_VERSION'), 'platform': sys.platform}, inputs=inputs, commands=commands,
        totals={status: sum(r['status'] == status for r in observed)
                for status in ('passed', 'failed', 'skipped', 'not-run')}, scenarios=records)
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / 'results.json').write_text(json.dumps(output, indent=2) + '\n')
    def esc(value): return html.escape(str(value), quote=True)
    rows = []
    for record in records:
        link = links.get(record.get('input', ''))
        evidence = f'<a href="{esc(link)}">Playwright HTML (screenshots/traces)</a>' if link else ''
        rows.append('<tr>' + ''.join(f'<td>{esc(record[key])}</td>' for key in ('section', 'scenario', 'lane', 'status', 'source')) + f'<td>{evidence}</td></tr>')
    page = ('<!doctype html><html lang="en"><meta charset="utf-8"><title>Kubarr test ledger</title>'
            '<style>body{font:16px system-ui;max-width:1200px;margin:auto;padding:2em;background:#101b2b;color:#eef}'
            'table{border-collapse:collapse;width:100%}td,th{padding:.5em;border-bottom:1px solid #536078;text-align:left}'
            'a{color:#9df}</style>'
            f'<h1>Kubarr test ledger</h1><p>Generated {esc(now)} · commit {esc(commit)} · '
            f'CI {esc(output["environment"]["ci"])} · run {esc(output["environment"]["run_id"] or "local")}</p>'
            f'<p>Inputs: {esc(", ".join(inputs) or "none")}. Missing lanes have no results; inventory-only requirements are not-covered.</p>'
            f'<p>Observed totals: {esc(output["totals"])}</p>'
            '<p><a href="results.json">Machine-readable results</a></p>'
            '<table><thead><tr><th>Section</th><th>Scenario</th><th>Lane</th><th>Status</th><th>Source</th><th>Evidence</th></tr></thead><tbody>'
            + ''.join(rows) + '</tbody></table></html>')
    (args.output / 'index.html').write_text(page)


if __name__ == '__main__': main()
