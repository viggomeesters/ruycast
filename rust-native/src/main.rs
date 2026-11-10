mod keyboard_environment;
mod main_loop;
mod native;
#[cfg(feature = "test-tools")]
use ruycast_native::native_f1;
mod native_dashboard;
mod native_settings;
mod native_menu_bar;
mod native_choice;
mod native_edit;
mod native_edit_menu;
mod native_editor;
mod native_snippet_editor;
mod native_keyboard;
mod native_library;
mod native_model;
mod native_monitor;
mod native_rows;
mod native_notice;
mod native_os_controls;
mod native_paste;
mod native_result;
mod native_services;
mod native_clipboard;
mod native_clipboard_preferences;
mod native_surface;
mod native_template;
use ruycast_native::session;

#[cfg(feature = "test-tools")]
#[used]
static OWNED_HOST_SINK: ruycast_native::profile_io_abi::HostSink =
    ruycast_native::owned_host_entry::ruycast_owned_host_bind_v1;

#[cfg(feature = "test-tools")]
#[used]
#[no_mangle]
#[link_section = "__DATA_CONST,__ruycast_input"]
pub static RUYCAST_NATIVE_INPUT_CALLBACKS_V1: ruycast_native::owned_host_entry::NativeInputCallbacksV1 =
    ruycast_native::owned_host_entry::NativeInputCallbacksV1 {
        magic: 0xF1000026494E5054,
        abi: 1,
        count: 5,
        carbon_pressed: ruycast_native::carbon_hotkeys::callback_entrypoints(),
        hid_input: ruycast_native::hid_service::callback_entrypoints().0,
        hid_removed: ruycast_native::hid_service::callback_entrypoints().1,
        hid_filter: ruycast_native::hid_service::callback_entrypoints().2,
        keyword_filter: ruycast_native::keyword_capture::callback_entrypoints(),
    };

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    #[cfg(feature = "test-tools")]
    if args.first().map(String::as_str) == Some("__f1-owned-host") {
        if args.len() != 1 {
            std::process::exit(86);
        }
        let host = match ruycast_native::owned_host_entry::take_bound_host() {
            Ok(host) => host,
            Err(_) => std::process::exit(86),
        };
        // The constructor must already have admitted the complete source/SDK
        // graph. Exercise the production service graph with its attached IO.
        match native::run_owned(host) {
            // The owned endpoint writes its framed protocol itself. Ordinary
            // runtime JSON here would be an extra, invalid protocol message.
            Ok(_) => {},
            Err(_) => std::process::exit(86),
        }
        return;
    }
    let result = match args.first().map(String::as_str) {
        None => production_paths().and_then(|paths|native::run(paths,false)),
        Some("--version") if args.len() == 1 => Ok(serde_json::json!({"ok":true,"result":{"name":"Ruycast","version":rustcast_core::product_version::CURRENT,"implementation":"rust","component":"host","localPreview":cfg!(feature = "local-preview")}})),
        Some("--set-config-directory") if args.len() == 2 => production_paths().and_then(|paths| relocate_config(paths, &args[1])),
        Some("--native") if args.len()==1 => production_paths().and_then(|paths|native::run(paths,false)),
        Some("--native-hidden") if args.len()==1 => explicit_paths().and_then(|paths|native::run(paths,true)),
        Some("--headless") if args.len()==1 => production_paths().and_then(|paths| native_services::headless(paths, false)),
        #[cfg(feature = "test-tools")]
        Some("--headless-test") if args.len()==1 => explicit_paths().and_then(|paths| native_services::headless(paths, true)),
        #[cfg(feature = "test-tools")]
        Some("--check-owned-clipboard") if args.len()==3 => native_services::check_owned_clipboard(&args[1], &args[2]),
        Some("--check-profile-native") if (2..=5).contains(&args.len()) => {
            let width = if args.get(3).map(String::as_str)==Some("600") {600.}else{760.};
            explicit_paths().and_then(|paths|native::check_profile(paths,std::path::Path::new(&args[1]),width,args.get(2).map(String::as_str)!=Some("light"),args.get(4).map(String::as_str).unwrap_or("")))
        },
        Some("--check-native") if args.len() <= 4 => {
            let width = match args.get(3).map(String::as_str) { None | Some("760") => 760., Some("600") => 600., _ => { eprintln!("Expected width 600 or 760"); std::process::exit(2) } };
            native::check(args.get(1).map(std::path::Path::new), width, args.get(2).map(String::as_str) != Some("light"))
        },
        Some("--benchmark-model") if args.len() == 1 => Ok(benchmark()),
        _ => Err("Unknown arguments. Start Ruycast without arguments, or use --headless or --version.".into()),
    };
    match result {
        Ok(value) => println!("{value}"),
        Err(message) => {
            println!("{}", serde_json::json!({"ok":false,"error":message}));
            std::process::exit(1);
        }
    }
}

fn explicit_paths() -> Result<ruycast_native::profile::ProfilePaths, String> {
    selected_paths(ruycast_native::startup::Policy::ExplicitProbe)
}
fn production_paths() -> Result<ruycast_native::profile::ProfilePaths, String> {
    if cfg!(feature = "local-preview") {
        let home = std::path::PathBuf::from(objc2_foundation::NSHomeDirectory().to_string());
        let root = home.join("Library/Application Support/Ruycast Rust Preview");
        return Ok(ruycast_native::profile::ProfilePaths::new(root, home, None));
    }
    selected_paths(ruycast_native::startup::Policy::Production)
}
fn relocate_config(paths:ruycast_native::profile::ProfilePaths,directory:&str)->Result<serde_json::Value,String>{
    // Validate authority before opening storage. An override must not even
    // create a lock or initialize a source as a side effect of this rejection.
    if paths.config_override.as_deref().is_some_and(|v|!v.is_empty()) {
        return Err("RUYCAST_CONFIG_DIR controls this configuration. Change or unset it first.".into());
    }
    paths.absolute(directory)?;
    let mut store=ruycast_native::profile::ConfigStore::open_existing(paths)?;
    let receipt=store.relocate(directory)?;
    Ok(serde_json::json!({"ok":true,"result":{"directory":store.file.parent(),"path":store.file,"originalRetained":true,"durable":receipt.durable,"warning":receipt.warning}}))
}
fn selected_paths(policy:ruycast_native::startup::Policy)->Result<ruycast_native::profile::ProfilePaths,String>{
    let environment=ruycast_native::startup::Environment{
        profile:std::env::var_os("RUYCAST_HOME"),legacy_profile:std::env::var_os("RUSTCAST_HOME"),home:std::env::var_os("HOME").map(std::path::PathBuf::from),
        config:config_environment("RUYCAST_CONFIG_DIR")?,legacy_config:if std::env::var_os("RUYCAST_CONFIG_DIR").is_none(){config_environment("RUSTCAST_CONFIG_DIR")?}else{None},
    };
    // The OS home lookup is production-only. Probe planning never consults it
    // and cannot silently adopt a personal profile if its marker is omitted.
    let fallback=if policy==ruycast_native::startup::Policy::Production&&environment.home.as_ref().is_none_or(|p|p.as_os_str().is_empty()) {
        Some(std::path::PathBuf::from(objc2_foundation::NSHomeDirectory().to_string()))
    }else{None};
    ruycast_native::startup::paths(&environment,&std::env::current_dir().map_err(|e|e.to_string())?,fallback.as_deref(),policy)
}
fn config_environment(key:&str)->Result<Option<String>,String>{
    std::env::var_os(key).map(|value|value.into_string().map_err(|_|"Choose a UTF-8 configuration folder path.".into())).transpose()
}

fn benchmark() -> serde_json::Value {
    use std::hint::black_box;
    use std::time::Instant;
    let terms = [
        "m",
        "mi",
        "mic",
        "microsoft",
        "alias42",
        "left",
        "project dashboard",
        "missing Ω",
        "",
        "notes",
    ];
    let mut queries = serde_json::Map::new();
    let mut checksum = 0usize;
    let mut sequence = 0;
    for size in [500, 1000, 5000, 10000] {
        let mut session = session::Session::new(session::fixture(size));
        let mut values = Vec::with_capacity(1000);
        for i in 0..1010 {
            let start = Instant::now();
            sequence += 1;
            session.set_query(black_box(terms[sequence % terms.len()]));
            checksum += black_box(session.rows().count());
            if i >= 10 {
                values.push(start.elapsed().as_secs_f64() * 1000.);
            }
        }
        values.sort_by(f64::total_cmp);
        queries.insert(
            size.to_string(),
            serde_json::json!({"samples":1000,"medianMS":values[500],
            "p95MS":values[949],"p99MS":values[989],"maxMS":values[999]}),
        );
    }
    serde_json::json!({"ok":true,"optimized":!cfg!(debug_assertions),"queries":queries,"checksum":checksum,
        "scope":"Rust session plus shared existing kernel, first slice only; no recency, analytics, complete GUI or physical display comparison"})
}
