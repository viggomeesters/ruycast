//! Rust installer. Normal execution changes the installation; tests inject all effects.
#[path = "ruycast-install/mod.rs"]
mod installer;
fn main() {
    if let Err(error) = installer::cli() {
        println!("{}", serde_json::json!({"ok":false,"error":error}));
        std::process::exit(1);
    }
}
