#[path = "../../rust-native/src/native_thread_boundary.rs"]
mod native_thread_boundary;
use objc2::MainThreadMarker;

fn main() {
    let main_thread = MainThreadMarker::new().unwrap();
    native_thread_boundary::spawn_runtime(move || drop(main_thread))
        .unwrap()
        .join()
        .unwrap();
}
