#!/usr/bin/env python3
"""Reconcile the shipped Swift surface with Rust source/tests, without claiming
parity from filenames or mentions. This inventories compiler-parsed entrypoints;
semantic contracts and acceptance remain explicit review work.
"""
import argparse
import ast
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess

REPO=Path(__file__).resolve().parents[2]

def group(id,files,rust,tests,contract,remaining):
    return dict(id=id,files=files.split(),rust=rust.split(),tests=tests.split(),contract=contract,remaining=remaining)

GROUPS=[
    group('shared-domain-kernel','RustBridge','','foundation keyboard template snippet','Reuse existing typed Rust validation, shortcut grammar, search/history/geometry with current serialization/CLI compatibility; remove unnecessary C-ABI/JSON inside final Rust process.','Complete typed crosswalk and source-bound parity for every current domain API.'),
    group('catalog-ranking','CommandProjection CommandSearchIndex SearchRanking','catalog ranking session','applications runtime','Stable catalog IDs, enabled/scoped filtering, aliases/Unicode/typo behavior, complete ordered recency and rank ties.','Full ranked system-provider/main Safari merge, whole-product parity and equivalent scale measurements.'),
    group('profile-configuration','Files Models ConfigurationLocation','profile private_storage config_effects configuration_jobs','foundation runtime inline','Current profile aliases/environment/portable mapping, ACL-aware private publication, last-good config, external edits, atomic/durable outcomes and staged runtime effects.','Full configuration-field/runtime/CLI matrix, all new native effects and final relocation/readback parity.'),
    group('transport-runtime','ControlServer ReplySink LockedValue BackgroundWork MainRunLoop FeatureState AppState','control runtime connected supervision main_services','runtime bridge cli_open','One authority for GUI/CLI; leases through physical retirement, bounded admission/frames/deadlines, cancellation, callbacks/main-thread isolation and ordered adopted snapshots.','Every wire operation, shutdown/launch modes, optional adapters and complete retained-state/lifecycle resource acceptance.'),
    group('launcher-native-ui','Palette PaletteGeometry PaletteSearchField LauncherResults Appearance AppBrand CommandGlyph CommandIcons WebsiteIcons ShortcutBadge NativeEditCommands','native native_model native_surface presentation recorder','bridge native applications','Native placement/focus/theme, keyboard/IME/mouse/scroll/selection, icons/groups/badges/loading/empty/error and native Edit/Undo/Redo compatibility including0.8.26.','All visual/placement/menu-bar/permission/provider states and excluded physical/display/Spaces/accessibility evidence remain separate.'),
    group('actions-bindings','PaletteActions ShortcutRecorder','actions native_surface keyboard_service','actions input_actions native_keyboard','Selected-ID Actions, direct Set Alias/Hotkey, conflict/clear/cancel/save, independent recording, metadata/privacy and dirty-draft lifetime.','Full provider-specific actions and remaining hook/Hyper interaction; physical focus/hardware behavior unpassed.'),
    group('management-editors','Management CommandEditor CaptureTextEditor SnippetTextEditor','library native_editor native_library native_template','editor creation library inline capture','Saved/discovered-command management, full fields and merging, inline transitions/typing/Undo/composition/cancel, rich snippet/link/line-number controls and capture.','All general Settings/permission controls, rich snippet editing and complete baseline editor behavior; not just current forms.'),
    group('scripts-choices-input','ScriptChoices TaskDateInput ScriptChoiceView Templates','execution script_result script_session templates template_form','script template input rich script_result','Literal arguments, deadlines/output/process groups, provider choices/details, input/date/project fields, owned cancellation and logs.','All builtin/private-provider/date and selection contracts, all CLI/GUI entries and complete error/timeout presentation.'),
    group('clipboard-snippet-output','SnippetOutput SnippetClipboard','snippet_output snippet_clipboard paste native_paste','snippet snippet_effects','Plain/Teams/HTML/labeled links, cursor positions, prepare/write/own-capture/hide/target/paste ownership and partial/unknown outcomes.','Final native/permission acceptance, keyword integration and complete app data compatibility; ordinary expansion preserves clipboard.'),
    group('global-keyboard','KeyboardState Hotkeys HyperKey','hotkeys keyboard_service carbon_hotkeys keyboard_state native_keyboard','keyboard native_keyboard hid_state','Staged Carbon/sided/modifier/Hyper, physical devices, secure input, permission/session/tap recovery, Caps restoration and recorder arbitration.','HID/Quartz SDK service, partial-failure resource retirement, accepted-generation integration and physical boundaries; pure state differential is not service acceptance.'),
    group('keyword-expansion','SnippetMatcher SnippetExpander SnippetChoice','','snippet','Suffix/keyword matching, deletion/replacement, choices, variables, secure typing, generated-event tags and teardown.','Keyword observer/matcher/choices/focus expansion integration and all error/resource/privacy paths are still required.'),
    group('clipboard-history','ClipboardPreview ClipboardSearch ClipboardStore','','','Capture/change tracking/source exclusions/private types/pause, text/image/file storage, dedup/retention/assets, async latest search, preview/copy/paste/delete/import/export transactions.','Complete native capture/history/backend/UI and current data/CLI compatibility.'),
    group('system-provider-session','SystemSearchSession SystemServices','','','Bounded demand-only provider requests, cancellation/absolute deadlines, command identity stores, fresh effect consent and no late selection substitution.','All provider integration and lifetime contracts, including Search Files/mdfind scopes, literal escaping/control-character rejection, bounded output and cancellation.'),
    group('window-layout','WindowPlacement WindowRead WindowManager','','','Current layouts/displays/restore state, AX role/process/object identity, minimized restoration and bounded retry without same-title fallback.','Full native window-layout integration and eligible isolated SDK/model tests; excluded physical display tests unpassed.'),
    group('window-search','WindowSearch','','','Window discovery/scoping, stable actionable identity, token lifetime and revalidation.','Dedicated window-search port; issue85 main-search feature remains separate.'),
    group('menu-search','MenuActions SystemAccessibility','','','Frontmost app menu path/title/shortcut/enabled state, accessibility identity and revalidation before execution.','Complete native menu provider and action port.'),
    group('safari-tabs','BrowserTabs','','','Safari demand-only discovery, main/dedicated search, separate token stores, absolute deadlines, cancellation, permission policy and documented identity limits.','Complete native Safari provider/main merge. Issue83 exact-alias priority remains separate.'),
    group('apple-shortcuts','AppleShortcuts','','','UUID discovery/execution, duplicate labels, saved shortcut binding and privacy.','Complete native provider and preference parity.'),
    group('utilities-secrets','Utilities','','','Saved/text/username histories, safe password generation, same Keychain service/account, concealed pasteboard types and no secrets in logs.','Complete utility providers, imports, data/security/CLI and UI integration.'),
    group('processes-monitor','SystemMonitor ResourceSampler MonitorPerformance ProcessIcon','','','Process/CPU/memory/wakeup snapshots, process ordering/actions/confirmation and icons; worker-bound bounded sampling.','Complete monitor/process provider/UI and performance telemetry.'),
    group('analytics-logs','AnalyticsStore AnalyticsDashboard CommandLog','activity','execution script_result','Actual command logs, usage/error/timing storage, summaries, retention/disable/export/reset and privacy.','Full analytics persistence/dashboard/wire parity and end-to-end timings, beyond current command logs/usage.'),
    group('f1-resource','PerformanceTrace PerformanceSoak','','','Equivalent full workloads, startup/build/size/idle/fault timing,10k checkpoints and settled memory growth with original budgets.','Full comparable optimized product measurements; model/component evidence is not whole-app gain.'),
    group('startup-release','ProductVersion PublicPreview main','','','Bundle/version ownership, launch/menu-bar/CLI discovery, privacy export, Rust-only packaging and source release/checksum/install/signing/config/permissions readback.','Complete build without Swift production dependency, reviewed public delivery/install/readback; retain current app until accepted.'),
]

def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def parse_module(module,paths,sdk,plugins):
    all_paths=sorted((REPO/'Sources'/module).glob('*.swift'))
    cmd=['swiftc','-frontend','-dump-ast','-whole-module-optimization','-sdk',sdk,'-swift-version','6','-module-name',module,'-I','.build/swift/debug/Modules','-I','.build/swift/debug/CRuycastCore.build']
    cmd+=plugins
    if module!='RuycastApp':cmd+=['-parse-as-library']
    for p in all_paths:
        if p in paths:cmd+=['-primary-file',str(p.relative_to(REPO))]
        else:cmd+=[str(p.relative_to(REPO))]
    run=subprocess.run(cmd,cwd=REPO,capture_output=True,text=True,timeout=90)
    if run.returncode:raise RuntimeError(run.stderr)
    seen=set(re.findall(r'\(source_file "([^"\n]+)"',run.stdout,re.MULTILINE))
    expected={str(p.relative_to(REPO)) for p in paths}
    if seen!=expected:raise RuntimeError('Compiler did not produce all requested source ASTs: '+str(expected-seen))
    declarations={name:[] for name in expected}
    pattern=r'\((func_decl|constructor_decl|destructor_decl|var_decl|class_decl|struct_decl|enum_decl|protocol) [^\n]*?range=\[([^\]]+)\][^\n]*?"([^"\n]+)"'
    for match in re.finditer(pattern,run.stdout):
        source=match[2].split(':',1)[0]
        if source in declarations:declarations[source].append(dict(kind=match[1],range=match[2],name=match[3]))
    for path,items in declarations.items():
        declarations[path]=list({(item['kind'],item['range'],item['name']):item for item in items}.values())
    context={str(p.relative_to(REPO)):sha(p) for p in all_paths}
    return declarations,dict(command=cmd,inputHashes=context,exitCode=run.returncode,diagnostics=run.stderr.strip(),sourceASTs=len(seen))
def test_methods(path):
    if not path.exists():return []
    tree=ast.parse(path.read_text());return sorted(n.name for n in ast.walk(tree) if isinstance(n,(ast.FunctionDef,ast.AsyncFunctionDef)) and n.name.startswith('test_'))

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--reference',type=Path,required=True);parser.add_argument('--out',type=Path,required=True);args=parser.parse_args()
    paths=[p for p in sorted((REPO/'Sources').rglob('*.swift')) if not any('Checks' in x for x in p.relative_to(REPO).parts)]
    reference=[p for p in sorted((args.reference/'Sources').rglob('*.swift')) if not any('Checks' in x for x in p.relative_to(args.reference).parts)]
    if {str(p.relative_to(REPO)) for p in paths}!={str(p.relative_to(args.reference)) for p in reference}:raise RuntimeError('Production source inventories differ from accepted release checkout')
    sdk=subprocess.check_output(['xcrun','--show-sdk-path'],text=True).strip()
    driver=subprocess.run(['swiftc','-###','-typecheck','Sources/RuycastApp/FeatureState.swift'],cwd=REPO,capture_output=True,text=True,check=True)
    driver_args=shlex.split(driver.stdout+driver.stderr);plugins=[]
    for i,arg in enumerate(driver_args):
        if arg in ['-plugin-path','-external-plugin-path','-load-plugin-library','-load-plugin-executable']:plugins+=driver_args[i:i+2]
    parsed={};compiler_context=[]
    for module in sorted({p.parent.name for p in paths}):
        declarations,context=parse_module(module,[p for p in paths if p.parent.name==module],sdk,plugins)
        parsed.update(declarations);compiler_context.append(context)
    modules={};owners={name:g['id'] for g in GROUPS for name in g['files']};duplicate=[name for g in GROUPS for name in g['files'] if sum(name in x['files'] for x in GROUPS)!=1]
    if duplicate:raise RuntimeError('Ambiguous module ownership: '+str(duplicate))
    for path in paths:
        name=path.stem;relative=str(path.relative_to(REPO));ref=args.reference/relative
        if name not in owners:raise RuntimeError('Unclassified production module: '+relative)
        if sha(path)!=sha(ref):raise RuntimeError('Reference source differs: '+relative)
        modules[relative]=dict(sha256=sha(path),contractFamily=owners[name],declarations=parsed[relative])
    groups=[]
    for g in GROUPS:
        row=dict(g);row['files']=[p for p,v in modules.items() if v['contractFamily']==g['id']]
        row['rustSources']=[str(p.relative_to(REPO)) for name in g['rust'] for p in [REPO/'rust-native/src'/f'{name}.rs'] if p.exists()]
        row['testCandidates']={str(p.relative_to(REPO)):test_methods(p) for name in g['tests'] for p in [REPO/'rust-native/tests'/f'{name}_process.py'] if p.exists()}
        row['acceptance']='Not accepted as a complete family. Candidate source/test crosswalk; compiler inventory and test names do not prove behavioral parity.'
        groups.append(row)
    app=(REPO/'Sources/RuycastApp/AppState.swift').read_text();handle=app[app.index('    func handle(_ req:[String:Any],sink:ReplySink)'):]
    ops=[]
    for line in handle.splitlines():
        if re.match(r'\s*case "',line):
            part=line.split(':',1)[0]
            ops.extend(re.findall(r'"([^"\n]+)"',part))
    native=(REPO/'rust-native/src/runtime.rs').read_text()
    wire=[dict(operation=op,production='Sources/RuycastApp/AppState.swift:handle(_:sink:)',nativeMentionLines=[i for i,line in enumerate(native.splitlines(),1) if '"'+op+'"' in line],acceptance='Unproven full wire parity; a mention or fallback stub is not actual entry/effect/response evidence.') for op in ops]
    head=subprocess.check_output(['git','-C',str(args.reference),'rev-parse','HEAD'],text=True).strip()
    result=dict(schema='ruycast.full-rust.behavior-matrix.candidate.v1',scope='Current accepted0.8.26 source crosswalk; production modules, compiler-parsed declarations, explicit semantic contract families, wire operations and candidate tests. Not final full behavior acceptance.',baseline=dict(version='0.8.26',build='66',referenceCommit=head,sourceOnlyComparison=True,releaseDeliveryEvidence='.go/evidence/native-edit-shortcuts/delivery-review.json'),compilerVersion=subprocess.check_output(['swiftc','--version'],text=True).strip(),compilerContext=compiler_context,importedModuleHashes={p:sha(REPO/p) for p in ['.build/swift/debug/Modules/LauncherCore.swiftmodule','.build/swift/debug/Modules/LauncherModels.swiftmodule','.build/swift/debug/CRuycastCore.build/module.modulemap']},sourceFiles=modules,contractFamilies=groups,wireOperations=wire,unclassifiedModules=[],allProductionSourcesMatchReference=True,acceptance='Pending independent semantic completeness review and criterion-level proof. No whole original checklist block is checked.',original12Retained=True,overallComplete=False)
    args.out.parent.mkdir(parents=True,exist_ok=True);args.out.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(dict(modules=len(modules),declarations=sum(len(m['declarations']) for m in modules.values()),families=len(groups),wireOperations=len(wire),unclassified=0,overallComplete=False)))
if __name__=='__main__':main()
