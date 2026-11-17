#[path = "../../rust-native/src/native_thread_boundary.rs"]
mod native_thread_boundary;
use objc2::MainThreadMarker;

fn main() {
    let _main_thread =
        native_thread_boundary::MainThreadState::new(MainThreadMarker::new().unwrap());
    native_thread_boundary::spawn_runtime(move || drop(String::from("owned worker value")))
        .unwrap()
        .join()
        .unwrap();
}
