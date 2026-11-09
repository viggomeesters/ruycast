//! Scalar, nonprompting permission queries. Owned fixtures replace the entire
//! source before a status request can call any actual platform API.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub accessibility: bool,
    pub input_monitoring: bool,
    pub raycast_running: bool,
}
#[derive(Clone, Copy)]
struct Api {
    accessibility: fn() -> bool,
    input_monitoring: fn() -> bool,
    raycast: fn() -> bool,
}
impl Api {
    fn read(self, raycast: bool) -> Snapshot {
        Snapshot {
            accessibility: (self.accessibility)(),
            input_monitoring: (self.input_monitoring)(),
            raycast_running: raycast && (self.raycast)(),
        }
    }
}
pub struct Source {
    api: Api,
    owned: Option<Snapshot>,
}
impl Source {
    pub fn new(fixture: bool) -> Self {
        Self {
            api: if fixture {
                Api {
                    accessibility: denied,
                    input_monitoring: denied,
                    raycast: denied,
                }
            } else {
                Api {
                    accessibility,
                    input_monitoring,
                    raycast: raycast_running,
                }
            },
            owned: fixture.then(Snapshot::default),
        }
    }
    pub fn read(&self, raycast: bool) -> Snapshot {
        if let Some(owned) = self.owned {
            return owned;
        }
        self.api.read(raycast)
    }
    #[cfg(feature = "test-tools")]
    pub fn set_fixture(&mut self, value: Snapshot) -> bool {
        if let Some(owned) = &mut self.owned {
            *owned = value;
            true
        } else {
            false
        }
    }
}
fn denied() -> bool {
    false
}
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
}
fn accessibility() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}
fn input_monitoring() -> bool {
    unsafe { CGPreflightListenEventAccess() }
}
fn raycast_running() -> bool {
    // NSRunningApplication is documented thread-safe in the SDK class contract.
    // No instance/title/process data crosses the scalar permission boundary.
    objc2::rc::autoreleasepool(|_| {
        !objc2_app_kit::NSRunningApplication::runningApplicationsWithBundleIdentifier(
            &objc2_foundation::NSString::from_str("com.raycast.macos"),
        )
        .is_empty()
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn yes() -> bool {
        true
    }
    fn no() -> bool {
        false
    }
    fn forbidden() -> bool {
        panic!("unexpected native or app lookup")
    }
    #[test]
    fn complete_injected_table_reports_each_permission_independently() {
        for accessibility in [false, true] {
            for monitoring in [false, true] {
                let api = Api {
                    accessibility: if accessibility { yes } else { no },
                    input_monitoring: if monitoring { yes } else { no },
                    raycast: forbidden,
                };
                let value = api.read(false);
                assert_eq!(value.accessibility, accessibility);
                assert_eq!(value.input_monitoring, monitoring);
                assert!(!value.raycast_running);
            }
        }
        assert!(
            Api {
                accessibility: yes,
                input_monitoring: no,
                raycast: yes
            }
            .read(true)
            .raycast_running
        );
    }
    #[test]
    fn owned_source_returns_before_constructing_or_calling_native_table() {
        let source = Source {
            api: Api {
                accessibility: forbidden,
                input_monitoring: forbidden,
                raycast: forbidden,
            },
            owned: Some(Snapshot {
                accessibility: true,
                input_monitoring: false,
                raycast_running: true,
            }),
        };
        let value = source.read(true);
        assert!(value.accessibility);
        assert!(!value.input_monitoring);
        assert!(value.raycast_running);
    }
}
