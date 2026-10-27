#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
directory="${RUYCAST_CHECK_OUTPUT:-.build/performance}"
mkdir -p "$directory"
 .build/swift/release/RuycastApp --benchmark-interactions > "$directory/interactions-release.json"
clang -O2 -Wall -Wextra scripts/perf-probe.c -o .build/perf-probe
.build/perf-probe --self-check > "$directory/counter-units.json"
.build/swift/release/RuycastApp --check-performance-pipeline > "$directory/pipeline-release.json"
.build/swift/release/RuycastApp --benchmark-performance > "$directory/benchmark-release.json"
.build/swift/release/RuycastApp --benchmark-lifecycle 100 > "$directory/lifecycle-release.json"
python3 - "$directory" <<'PY'
import json,sys
from pathlib import Path
root=Path(sys.argv[1]);b=json.loads((root/'benchmark-release.json').read_text());life=json.loads((root/'lifecycle-release.json').read_text())
assert b['ok'] and b['releaseBuild'] and life['ok']
assert json.loads((root/'interactions-release.json').read_text())['ok']
assert b['queries']['500']['p95MS']<=5, '500-command model budget exceeded'
assert b['queries']['5000']['p95MS']<=15, '5000-command model budget exceeded'
assert b['processResultReads1000']['p95MS']<=.5, 'Process reads repeat expensive work'
assert b['clipboardSearchEnqueue200x8kB']['p95MS']<=5, 'Clipboard query blocks input'
assert life['cycles']==10000 and life['growthMiB']<=10, '10000-cycle settled footprint budget exceeded'
start=life['startFootprintBytes']
checkpoints={r['cycles']:r['footprintBytes'] for r in life['samples']}
for cycles in (1000,5000,10000):
    assert (checkpoints[cycles]-start)/1048576<=10, f'{cycles}-cycle retained footprint budget exceeded'
print(json.dumps({'ok':True,'scope':'Release model/native fixtures; no physical display, hotkey or energy claim','cycles':life['cycles'],'growthMiB':life['growthMiB']}))
PY
