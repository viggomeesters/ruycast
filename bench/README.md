# F1 source staging

The existing runner, full SDK source basis and historical reference sources are
staged here for review. This is an incomplete portability integration, not a
reproducible or admitted benchmark yet. Ordinary product builds do not depend
on this directory.

## Source destinations

- `f1-runner/Cargo.toml`, `Cargo.lock` and its explicitly selected existing
  `src/` files retain the runner, collector, Guardian, private-cap transport and
  SDK in one package. Its native dependency remains `../../rust-native`.
- `reference/SwiftF1Bridge.swift` and `reference/ReferencePreentry.rs` retain the
  declared reference ABI. The latter is an admission object, not a complete
  reference endpoint implementation.
- `reference/0.8.26/` retains the specifically pinned reference `Package.swift`,
  `Sources/`, Rust core sources/manifests, default configuration and required
  test-source/fixture inputs. This is the historical oracle, not an alternative
  production build or a fallback to the installed application.

Only explicitly reviewed source files belong in this staging. Do not import a
workspace tree, `.go` state, `.build` contents, compiler objects, old receipts,
private profiles, generated grants, nonces, raw outputs or private history.
The public exporter must select each approved path explicitly.

## Changes required in the existing producer

The first relocation derives the checkout from the compiled manifest location,
limits inventory to that checkout or its pinned reference, redirects bootstrap
reference includes and confines endpoint artifacts to this checkout's existing
target. It does not modify any previously executed laboratory artifact.
The remaining producer work is:

1. Independently verify the relocated checkout, reference and endpoint path
   restrictions against exact source inventories and ownership requirements.
2. Keep Native and reference roots distinct. Resolve the historical oracle from
   `bench/reference/0.8.26`; build its core and reference objects from those
   bound sources. Never reuse old private object paths as public provenance.
3. Replace private source/argument/dependency evidence paths with source-owned
   relative input declarations and fresh compiler receipts. Verify the complete
   actual dep-info closure and exact compiler/rlib joins before grants. A new
   current-file hash alone is not authority for a cached rlib.
4. Preserve Foundation's two separate foreign declaration inputs: the selected
   SDK NSString header and the locked Cargo-generated NSString source. Resolve
   them through the actual selected SDK/compiler dependency receipt and bind
   their bytes, owner and declaration provenance. Do not accept an arbitrary
   external root or a relocated string as equivalent evidence.
5. Relocate SDK bootstrap reference-source includes to the pinned oracle, while
   retaining Native source includes against the shipping checkout. The same
   reviewed source identities must reach compiled pins, manifest and final
   private grant. Preserve the image/code, initializer and one-shot cap checks.
6. Generate output only in the caller's exclusive owned build directory. Keep
   private captures out of source inventory/export. Preserve the current time,
   size, permissions, EOF, cancellation and cleanup limits.

The full SDK package and the smaller Foundation/AppleEvent consumer have
different source compositions. Reconcile reviewed deltas explicitly; do not
replace the full SDK with the consumer tree. Historical receipts remain tied
to their original source and artifact, rather than being rewritten for this
layout.

No authority records have been copied from laboratory evidence. The canonical
Data recipe now builds and binds fresh Native dependencies, SDK metadata and
final child inputs. The `prepare-foundation-data` and `prepare-apple-event-data`
commands compile and prepare without starting the endpoint; `run-*` commands
execute it and require separate qualified admission. Missing source, compiler,
artifact or capability bindings still refuse. This is not full SDK or F1
acceptance, and historical laboratory paths are not a public compiler recipe.

## Private preparation inputs

`prepare-owned-host-images ROOT FIXTURE_JSON OUT` requires the exact generated
`fixture.json` path. The old `ROOT OUT` form is refused before compiler work;
there is no guessed sibling path or environment epoch. The private Native grant
binds both `fixture: {path, sha256}` and the identical `fixture_sha256`. The epoch
is read from that bounded, unchanged manifest, never from a free scalar.

`prepare-reference-pins ROOT MAIN FIXTURE_JSON A|B OUT` writes source only. It
creates genuine private SDK-cap/stdio bindings, four private runtime roots and
a profile inside the admitted home, plus a compile-time eleven-field
`ReferencePins` maker and its source manifest. Its cap expires when the command
returns: the saved source/receipt is not runnable or replayable authority.
The actual Reference SDK compilation must retain a fresh holder, use exactly
the existing nine dependencies with a genuinely matched NativeLIB/compiler
cohort, and pass only this SDK role `--cfg rcf1_reference_sdk --check-cfg
cfg(rcf1_reference_sdk)`. It emits no SDK constructor; the separate Main keeps
its one qualified preentry initializer. No Native capability is compiled or
inherited by the Reference entry.

The fixed `sdk::reference_runtime::assemble_reference` actor builder must exist
and construct the real admitted workspace, factories, catalog and all required
module attestations. No placeholder assembler is generated. Complete actor
assembly, actual nine-slot/foreign compiler joins and the static25-Main/five-SDK
code pins remain required before a runnable Reference publication.

`build-reference-full-host-interface SNAPSHOT_ROOT BRIDGE EXISTING_BUILD_ROOT
INPUTS_JSON PINNED_SHA256 OUT` requires an explicit existing reference compiler
workspace and its path/SHA256 input-array receipt. Every frozen snapshot member
and the complete `Sources` set must match; selected objects, modules and archive
must also match the manifest and fresh before/after reads. It creates no sibling
workspace or fallback target. The optimized debug capture retains actual
compiler intermediates, dSYM/UUID, object DWARF and dependency records; missing
before inputs or typed objects refuse. These records do not grant callback or
Reference runtime authority. The partial `build-reference-interface ROOT BRIDGE
OUT` recipe retains its previous arguments.

The source-owned SDK producer resolves the locked Foundation package and selected
SDK, requires the actual same-build NSString dep-info/rlib/extern join, and bakes
the two declaration paths, hashes and owner IDs into the SDK. Cached rlibs
without that receipt refuse. The SDK uses bounded read-only file capabilities
and compares those compiled pins; runtime environment values cannot supply
them. Bootstrap tests require an explicit canonical, user-owned mode0700
`RUYCAST_NATIVE_TEST_FIXTURE_DIR` parent.

Earlier bounded producer pure checks compile and pass; the new private
ReferencePins/fixture-custody source requires its own scoped compiler review.
The full SDK syntax attempt with
the old Data Native rlib correctly refused its missing profile-I/O exports;
that dependency is not a current SDK compiler closure. A fresh source-quiet
Native release compiler receipt and matching dependency joins remain necessary
before a joint canonical SDK build. No portable export or Data preparation has
been run here.

## Protocol and admission

Keep the existing 157-lane definitions, reference ABI, fixture semantics,
ordered outputs, cold-launch requirements and completion/resource checks.
Staging source does not qualify missing workload handlers, provide a complete
reference coordinator, admit the fourteen SDK families, enable collection or
authorize an SDK/endpoint execution. Existing refusal gates stay closed until
their actual source and runtime requirements are independently satisfied.
