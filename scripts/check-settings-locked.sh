#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
directory="${RUYCAST_CHECK_OUTPUT:-.build/native-checks}"
mkdir -p "$directory"
.build/swift/debug/RuycastApp --check-settings "$directory" > "$directory/interaction.json"
.build/swift/debug/RuycastApp --check-interactions > "$directory/launcher-interaction.json"
.build/swift/debug/RuycastApp --check-actions-menu > "$directory/actions-interaction.json"
.build/swift/debug/RuycastApp --check-apple-shortcuts "$directory/apple-shortcuts" > "$directory/apple-shortcuts.json"
cat "$directory/apple-shortcuts.json"
.build/swift/debug/RuycastApp --check-window-search "$directory/window-search" > "$directory/window-search.json"
cat "$directory/window-search.json"
.build/swift/debug/RuycastApp --check-menu-actions "$directory/menu-actions" > "$directory/menu-actions.json"
cat "$directory/menu-actions.json"
.build/swift/debug/RuycastApp --check-render-updates "$directory/render-updates" > "$directory/render-updates.json"
cat "$directory/interaction.json" "$directory/launcher-interaction.json" "$directory/actions-interaction.json" "$directory/render-updates.json"
.build/swift/debug/RuycastApp --check-feedback "$directory/feedback" > "$directory/feedback-interaction.json"
cat "$directory/feedback-interaction.json"

.build/swift/debug/RuycastApp --check-rich-resources "$directory/rich-resources" > "$directory/rich-resources.json"
cat "$directory/rich-resources.json"

.build/swift/debug/RuycastApp --check-analytics "$directory/analytics" > "$directory/analytics.json"
cat "$directory/analytics.json"

.build/swift/debug/RuycastApp --check-performance-pipeline > "$directory/performance-pipeline.json"
cat "$directory/performance-pipeline.json"

.build/swift/debug/RuycastApp --check-visual-assets > "$directory/visual-assets.json"
cat "$directory/visual-assets.json"

.build/swift/debug/RuycastApp --check-command-log > "$directory/command-log.json"
cat "$directory/command-log.json"

.build/swift/debug/RuycastApp --check-polish "$directory/polish" > "$directory/polish.json"
cat "$directory/polish.json"
