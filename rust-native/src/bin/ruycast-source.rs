//! Clean local source export. This tool never invokes Git or publishes.
#[path = "ruycast-source/mod.rs"]
mod source;
fn main() {
    match source::cli(std::env::args().skip(1).collect()) {
        Ok(value) => println!("{value}"),
        Err(error) => {
            println!("{}", serde_json::json!({"ok":false,"error":error}));
            std::process::exit(1);
        }
    }
}
