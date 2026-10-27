# Security and data storage

Ruycast runs your configured commands with your user account. Its control interface is intended for you and your local tools.

## Stored on your Mac

The profile defaults to `~/.config/ruycast`, with owner-only directory permissions. Config, clipboard history, image assets and analytics use owner-only files. Clipboard contents are not encrypted by Ruycast; your Mac's storage protection applies.

Configuration can live in a separate folder selected through Settings or `ruycast config directory`. Only `config.json` is portable; history, logs, sockets and locks stay in the local profile. Configuration may contain private URLs, snippet text and script paths: keep your chosen folder outside the public source checkout. A synced folder uses your sync provider’s privacy and access controls. The previous config remains as a local recovery copy after relocation.

Clipboard history skips concealed, transient and generated pasteboard types, configured excluded apps, and app-switch intervals where attribution is uncertain. Passwords and supported 1Password bundle identifiers are excluded by default. History is bounded by item count, age and asset size. File copies retain references to the original files.

Analytics stores bounded local aggregates and recent fixed diagnostic categories. Command identifiers are hashed in its file. Queries, clipboard bodies, script arguments, stdout and stderr are excluded. Aggregate export removes command identifiers and event timelines. [Analytics details](docs/ANALYTICS.md).

Password-generator history uses macOS Keychain. Compatibility with the earlier Utilities history is optional at use time; no passwords or signing material are bundled.

The separate owner-only command log stores identifiers, execution metadata and redacted, bounded error text. It excludes successful output, inputs and clipboard data; provider error excerpts should be reviewed before sharing. It is not included in analytics export. [Command diagnostics](docs/DIAGNOSTICS.md).

Website icons make requests to website origins and discovered icon URLs. Quicklink credentials, paths, query strings and fragments are removed from the initial origin request. Fetches use a bounded downloader and local cache.

## Private writes and recovery

Native JSON storage, CLI configuration writes, analytics export files and the website icon cache use descriptor-based private writers. It first secures an empty staging directory (0700, no extended ACL), then creates the payload file with mode 0600 under that ACL-free directory. Content is written only after owner, mode and ACL checks; replacing the destination is atomic. Merely changing a file's permissions after creation would allow inherited ACL access or an earlier reader to retain a descriptor.

Existing selected portable-folder permissions and ACLs are preserved. The local profile is explicitly owner-only (0700, no extended ACL) before native state reads or control-server startup and before CLI configuration writes; supported profile aliases are retained. Configuration initialization and relocation publish with an atomic no-replace rename, so an existing file or symlink is preserved. The internal clipboard asset directory is kept owner-only with no inherited ACL before new image files are created. These measures do not encrypt content or revoke descriptors opened before the protection was installed. Sync services, deliberate later sharing and administrator access retain their own policies.

The destination filesystem must support these private permissions and ACL checks. Normal masks 000, 022 and 077 work; an unsearchable creation mask such as 0777 fails before payload creation and preserves the old destination. Normal failed writes remove their own temporary stage. A forced termination can leave a private `.ruycast-*.tmp` directory; unknown older temporary files are not automatically deleted. CLI configuration writes retain file and parent-directory synchronization; native create-only configuration publication also synchronizes. A synchronization error after publication can report failure after the destination has changed, so inspect the actual state before retrying. Atomic replacement is not a general claim of power-loss durability or protection against hostile processes running as the same user.

## Command and operating-system boundaries

The private Unix socket is restricted to the profile owner. It is not a network API or an authentication boundary against other processes running as your user.

Scripts receive literal arguments without shell interpolation, have deadlines and bounded captured output, and run in an owned process group. Only configure executables you trust. Process actions recheck the selected PID, owner and start time before acting.

Accessibility enables window control and text/key delivery. Ruycast does not edit macOS's permission database or grant itself access. Updates preserve the signing identity; changing your identity or moving to another distribution identity can require new permission approval.

## Reporting a problem

For a vulnerability, use the repository's private security reporting when available. If a private channel is not available, open an issue asking for a private contact without including credentials, clipboard contents or exploit details. For ordinary bugs, share the smallest reproducible example and an aggregate diagnostic export.
