# F1 source staging

The runner and its Rust SDK support are staged here for review. This is an
incomplete portability integration, not a reproducible or admitted benchmark
yet. Ordinary product builds do not depend on this directory.

## Source destinations

- `f1-runner/Cargo.toml`, `Cargo.lock` and its explicitly selected existing
  `src/` files retain the runner, collector, Guardian, private-cap transport and
  SDK in one package. Its native dependency remains `../../rust-native`.
- `reference/ReferencePreentry.rs` retains Rust reference-admission scaffolding;
  it is not a complete or active reference endpoint implementation. Its CLI
  routes refuse immediately.
- The historical `0.8.26` source tree has been removed. Its exact Git tree
  `349cc54484e6b210b1125f32f6417f275c51bbe8` is recoverable from commit
  `fff08adf6567ffe5ab2d8804c36c29483fa9390b`; it is not a current build or test
  input.

Only explicitly reviewed source files belong in this staging. Do not import a
workspace tree, `.go` state, `.build` contents, compiler objects, old receipts,
private profiles, generated grants, nonces, raw outputs or private history.
The public exporter must select each approved path explicitly.

## Changes required in the existing producer

The first relocation derives the checkout from the compiled manifest location,
limits inventory to that checkout, redirects bootstrap
reference includes and confines endpoint artifacts to this checkout's existing
target. It does not modify any previously executed laboratory artifact.
The remaining producer work is:

1. Independently verify the relocated checkout, reference and endpoint path
   restrictions against exact source inventories and ownership requirements.
2. Keep Native source inputs distinct from historical provenance. The retired
   Reference build route must not be restored from the removed source tree or
   old private object paths.
3. Replace private source/argument/dependency evidence paths with source-owned
   relative input declarations and fresh compiler receipts. Verify the complete
   actual dep-info closure and exact compiler/rlib joins before grants. A new
   current-file hash alone is not authority for a cached rlib.
4. Preserve Foundation's two separate foreign declaration inputs: the selected
   SDK NSString header and the locked Cargo-generated NSString source. Resolve
   them through the actual selected SDK/compiler dependency receipt and bind
   their bytes, owner and declaration provenance. Do not accept an arbitrary
   external root or a relocated string as equivalent evidence.
5. Reference SDK bootstrap compilation is retired. Any future reactivation
   requires newly reviewed source inputs and a separately qualified route.
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

The `prepare-reference-pins` command and its source-emission producer are
removed. Historical source-only receipts were never runnable or replayable
authority. No current runner command compiles or launches the Reference SDK. The historical
commands `prepare-reference-images`, `run-reference-assembly` and
`run-reference-input` fail before path access, output creation or process start.
The original fixed Swift declaration
roster is unavailable; recover its exact source before any future reactivation.
Do not reconstruct or substitute declaration bytes.

No current command builds that actor or publishes a runnable Reference image.
The retained-Main capture reader and its ignored capture test were also removed;
current code no longer reads that historical image. The uncalled fixed
`sdk::reference_runtime::assemble_reference` actor assembler has also been
removed, along with its uncalled workspace-model/bootstrap construction path.
The historical acceptance still required complete module
attestations, same-build compiler joins and static Main/SDK code pins; the
source-only pins receipt did not satisfy them.

The historical `build-reference-full-host-interface`,
`build-reference-interface`, `resume-reference-debug`,
`prepare-reference-images`, `run-reference-assembly` and
`run-reference-input` commands are retired.
The runner refuses them before path access or process start. Their old compiler
and debug-capture implementation has been removed; no Swift bridge or Reference
runtime authority is retained by this staging.

The SDK compiler/launch producer, source-join helpers and source-only pin
preparer have all been removed. No current command compiles or starts the
Reference SDK. Any future reactivation needs a new reviewed issue after the
exact original declaration roster has been recovered.

## Protocol and admission

Keep the existing 157-lane definitions, reference ABI, fixture semantics,
ordered outputs, cold-launch requirements and completion/resource checks.
Staging source does not qualify missing workload handlers, provide a complete
reference coordinator, admit the fourteen SDK families, enable collection or
authorize an SDK/endpoint execution. Existing refusal gates stay closed until
their actual source and runtime requirements are independently satisfied.
