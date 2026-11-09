// Disposable SDK boundary; all handles/state are local integers, never IOKit.
#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOKitLib.h>
#include <IOKit/hidsystem/IOHIDLib.h>
#include <IOKit/hidsystem/IOHIDParameter.h>
#include <IOKit/hidsystem/IOHIDShared.h>
#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>

#define ABI(fn, type) _Static_assert(__builtin_types_compatible_p(__typeof__(&(fn)), type), #fn " ABI")
ABI(IOServiceMatching, CFMutableDictionaryRef (*)(const char *));
ABI(IOServiceGetMatchingService, io_service_t (*)(mach_port_t, CFDictionaryRef));
ABI(IOServiceOpen, kern_return_t (*)(io_service_t, task_port_t, uint32_t, io_connect_t *));
ABI(IOObjectRelease, kern_return_t (*)(io_object_t));
ABI(IOServiceClose, kern_return_t (*)(io_connect_t));
ABI(IOHIDGetModifierLockState, kern_return_t (*)(io_connect_t, int, bool *));
ABI(IOHIDSetModifierLockState, kern_return_t (*)(io_connect_t, int, bool));
_Static_assert(sizeof(io_connect_t) == 4 && sizeof(kern_return_t) == 4 && sizeof(bool) == 1, "Rust scalar ABI");
_Static_assert(kIOHIDParamConnectType == 1 && kIOHIDCapsLockState == 1, "SDK constants");
static uint64_t failures;
static unsigned calls;
static bool latch, service, connection;
static unsigned hook_at;
static void (*hook)(void *);
static void *hook_context;
void ruycast_caps_fixture_hook(unsigned at, void (*callback)(void *), void *context) {
    hook_at = at; hook = callback; hook_context = context;
}
static int call(const char *name) {
#ifndef RUYCAST_HID_HOST_FIXTURE
    fprintf(stderr, "caps-fixture:%s\n", name);
#endif
    calls++;
#ifdef RUYCAST_HID_HOST_FIXTURE
    const char *home=getenv("RUYCAST_KEYBOARD_FIXTURE_HOME");
    if(home){char path[4096];const char *phase=strcmp(name,"set:true")==0?"restore":name;snprintf(path,sizeof(path),"%s/caps-fail-%s",home,phase);if(access(path,F_OK)==0)return -100;}
#endif
    if (hook && calls == hook_at) {
        void (*callback)(void *) = hook; hook = NULL; callback(hook_context);
    }
    return calls < 64 && (failures & (UINT64_C(1) << calls)) ? -(int)calls : 0;
}
uint64_t ruycast_caps_fixture_ready(void) { return UINT64_C(0x5255434150533031); }
void ruycast_caps_fixture_configure(bool original, uint64_t mask) {
    assert(!connection && !service); calls = 0; latch = original; failures = mask;
}
void ruycast_caps_fixture_failures(uint64_t mask) { failures = mask; }
uint32_t ruycast_caps_fixture_state(void) { return latch | (connection << 1) | (service << 2); }
CFMutableDictionaryRef ruycast_caps_fixture_matching(const char *name) {
    assert(strcmp(name, "IOHIDSystem") == 0);
    return call("matching") ? NULL : (CFMutableDictionaryRef)(uintptr_t)11;
}
io_service_t ruycast_caps_fixture_service(mach_port_t port, CFDictionaryRef dict) {
    assert(port == 0 && (uintptr_t)dict == 11);
    if (call("service")) return 0;
    assert(!service); service = true; return 12;
}
uint32_t ruycast_caps_fixture_task(void) { call("task"); return 13; }
kern_return_t ruycast_caps_fixture_open(io_service_t obj, task_port_t task, uint32_t type, io_connect_t *out) {
    assert(service && obj == 12 && task == 13 && type == kIOHIDParamConnectType && out && *out == 0);
    int result = call("open");
    // Undefined output on failure must never be treated as an owned handle.
    if (result) { *out = 99; return result; }
    assert(!connection); connection = true; *out = 14; return 0;
}
kern_return_t ruycast_caps_fixture_release(io_object_t obj) {
    assert(service && obj == 12); service = false; return call("release");
}
kern_return_t ruycast_caps_fixture_close(io_connect_t conn) {
    assert(connection && conn == 14); connection = false; return call("close");
}
kern_return_t ruycast_caps_fixture_get(io_connect_t conn, int selector, bool *out) {
    assert(connection && conn == 14 && selector == kIOHIDCapsLockState && out);
    int result = call("get"); if (!result) *out = latch; return result;
}
#ifndef OMIT_SET
kern_return_t ruycast_caps_fixture_set(io_connect_t conn, int selector, bool value) {
    assert(connection && conn == 14 && selector == kIOHIDCapsLockState);
    latch = value; // Failure can still have an effect.
    return call(value ? "set:true" : "set:false");
}
#endif
